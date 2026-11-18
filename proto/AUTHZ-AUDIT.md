# Authorization audit: every way in, the check it meets, the test that proves it (#161)

Part of the release gate (#152, DESIGN section 23, R35): no operation reaches the store without a `Principal` and the
guard, and credentials are verified on every request. This is the map. It lists every wire method, every command of
the CLI and every `klatch admin` command with the check each goes through and the test that fails if the check goes
away. Tests keep it honest: `tests/authz_audit.rs` fails when a wire method, a command or an admin command exists in
the source and is missing here, and the zero-trust test in the gateway fails when a public gateway method is not
classified.

**This measures that a decision is made; it is not a claim that the decisions are right, and nothing here is proven.**
Property tests, model checks and bounded proofs are separate (DESIGN item 66).

## The layers a request meets

1. **Identity.** The transport makes a `Principal` (`gateway/identity.rs`, `gateway/listener.rs`, `identity.rs`). A
   process holding a key bundle is `Verified` (its credential was live when the process started); a typed or
   discovered name is `Unattributed`. Nothing is trusted for its address or its process: the `Process` evidence is
   only ever mapped to `Unattributed`, proved by `gateway/listener.rs` tests and `tests/hook_identity.rs`.
2. **Credential liveness, on every request.** `Gateway::check_credential_live` (in `gateway/gateway/roles.rs`) runs
   first in `check_guard`: a `Verified` principal whose member holds an issued credential is refused (rule
   `credential`) once the credential is expired or revoked, with the clock read at each request. Proof:
   `worker_credentials.rs` (expiry boundary at the last second and the first second after it, revocation, other
   principals untouched, the breaker not counting it) and the wire test in `session/tests/wire.rs`. Gap: the
   own-record methods (#835).
3. **Roles.** `check_role` in the same function: librarian may only look (`read-only`), reviewer may only review
   (`reviewer`), admin agent has an allowlist (`admin-scope`, `floors`), members' admin acts need a role on a
   members' gateway, room creation and invite need the capability (`room-capability`). Proof: `admin_role.rs`,
   `librarian.rs`, `reviewer.rs`, `room_capability.rs`, `floors.rs`.
4. **Stops, runs, direct messages, clearance.** `authorize` adds, for an operation about a room: the breaker, a
   member removed from the room, a pause, a closed room, run rooms, direct-message membership and clearance against
   the room's label. Proof: `guarding.rs`, `labels.rs`, `dm.rs`, `runs.rs`, `tripwire.rs`.
5. **The guard.** `Guard::check` is asked once per operation (`the_guard_sees_every_guarded_method_exactly_once_per_call`).
   Every gateway an agent can reach is wrapped in `ForbidFloors`.
6. **Receiver budget and the scrubber** on content (`budget.rs`, `rate_limit.rs`, `scrub.rs`).

## Gateway methods

Every public method of `Gateway` is in one of two lists in `gateway/gateway/tests/zero_trust.rs`:

- `GUARDED`: asks the guard (and so the credential and role checks) first. The test
  `under_a_guard_that_refuses_everything_no_guarded_method_succeeds_or_touches_the_store` calls each one under a guard
  that refuses everything and fails if any succeeds or reaches the store; `the_guard_sees_every_guarded_method_exactly_once_per_call`
  fails if one does not ask the guard. The two review operations are refused by the role before the guard unless the
  caller is a reviewer, so `a_reviewer_is_still_refused_by_the_guard_for_both_review_operations` proves the guard for
  them.
- `OWN_RECORD`: builds the gateway, is a policy lookup that takes no principal, or acts only on the caller's own
  record, each with a written reason.

`every_public_method_of_the_gateway_is_classified` fails when a public method is in neither list. Before this audit
the list of scanned files was kept by hand and eight files were missing from it, so 24 public methods (among them
`admin_set_admin_agent`, `admin_set_librarian`, `admin_set_reviewer`, `admin_set_rate_limit`, `invite`,
`request_crossing`, `review_next` and `review_verdict`) were in no list. The test now reads the directory
(`gateway_sources`), and `the_scan_reads_every_source_file_of_the_gateway` names the files that were missing.

## Wire methods (PROTOCOL.md section 5)

Every request is dispatched in `session/methods.rs` (`handle`) or `session/mod.rs` (`request`), always as the
transport's principal; a `member`, `sender` or `from` param has no effect.

| Wire method | Gateway call | Operation the guard sees | Proof |
|---|---|---|---|
| `subscribe` | `head`, then `Subscription::poll` per tick (`tail`) | `Subscribe` (every poll) | `zero_trust.rs` GUARDED `head`, `tail`; `session/tests/live.rs` |
| `unsubscribe` | none (drops the session's own subscription) | none: no state of anyone else is touched | `session/tests/live.rs` |
| `post` | `post_labeled`, `post_handoff` | `CreateRoom` when new, `Post` | GUARDED `post`, `post_labeled`, `post_handoff`; `room_capability.rs` |
| `dm` | `dm` | `Post` in the pair room, after the pair check | GUARDED `dm`; `dm.rs` |
| `read` | `read` | `Read` (advance unless `peek`) | GUARDED `read`; `labels.rs` |
| `history` | `history` | `Read` without advance | GUARDED `history`; `history.rs` |
| `invite` | `invite` | `Invite`, then `Post` in `via` | GUARDED `invite`; `room_capability.rs` |
| `who` | `who` | `Who` | GUARDED `who` |
| `kind` | `set_kind` | none: the caller's own display word | OWN_RECORD `set_kind`; `member_kind.rs` |
| `status` | `set_status` | `Status` | GUARDED `set_status` |
| `claim` | `claim_ttl_for`, `claim` | `Claim` | GUARDED `claim`, `claim_ttl_for`; `claims.rs` |
| `scope` | `set_scope` | none: the caller's own scope | OWN_RECORD `set_scope`; gap #835 for credential liveness |
| `mute` | `set_muted` | `Mute` | GUARDED `set_muted`; `mute.rs` |
| `release` | `release`, `release_with_note` | `Release` | GUARDED `release`, `release_with_note` |
| `claims` | `claims` | `Claims` | GUARDED `claims` |
| `label_order` | `label_order` | `Admin("label-order")`, any member's, read-only (levels and cap, no rooms or clearances) | GUARDED `label_order`; `session/tests/dm.rs` |
| `scrub_check` | `scrub_check` | `Admin("scrub-check")`, any member's, yes or no only, never a rule name or term, no refusal row | GUARDED `scrub_check`; `session/tests/dm.rs` |
| `prune` | `prune_members` | `Admin("prune")`, every member's, fixed retention | GUARDED `prune_members`; `admin_role.rs` |
| `role_act` | the `admin_*` acts of the act named | `Admin(act)`, then the admin agent allowlist; floors refused | GUARDED `admin_close_room` and the rest; `session/role_tests.rs`, `floors.rs` |
| `review_next` | `review_next` | `ReviewNext`, reviewer role only | GUARDED `review_next`; `reviewer.rs` |
| `review_verdict` | `review_verdict` | `ReviewVerdict`, reviewer role only | GUARDED `review_verdict`; `reviewer.rs` |

The client frame `ack` calls `ack` (`Ack`, GUARDED `ack`). An unknown method answers `unknown_method` and reaches no
gateway call. The wire protocol has no floor act and no `admin` method of its own: `role_act` is the only way in and
it answers `unknown_act` for anything outside the admin agent's list. The floors have a read-only wire form only
(`label_order`, `scrub_check`, decision on #740): the scrub add, remove and list and the label chain and cap writes stay
terminal-only and answer `unknown_method` on the wire (`the_scrub_rules_and_the_label_order_writes_stay_off_the_wire`).

## CLI commands

Commands that act as a member go through `run_as_member`: the caller is identified first (a key bundle, else a
session found from the process tree or a typed name, never trusted for more than `Unattributed`), the gateway is
built with `ForbidFloors(AllowAll)` and `for_members()`, and every command calls the gateway through the `Direct`
transport with the caller's principal. So each of these meets exactly the checks of the wire method or gateway call it
wraps.

| Command | Gateway call | Check |
|---|---|---|
| `join` | `join`, or `join_with_harness` when the walk verified a Claude Code session (#877) | GUARDED `join`; plus the room's harness requirement, with the harness taken only from the verified session (never from the caller), and every other caller the generic floor (Codex too: no registry record to match, #884) |
| `dm` | `dm` | as wire `dm` |
| `post` | `post_labeled` | as wire `post` |
| `read` | `read` | as wire `read` |
| `history` | `history` | as wire `history` |
| `wait` | `wait_poll`, `wait_seen` | GUARDED `wait_poll`, `wait_seen` |
| `invite` | `invite` | as wire `invite` |
| `router` | `join`, `open_asks`, `claims`, `who`, `invite`, repeated by the command, as the caller's own member (#189) | GUARDED each of those calls; the router holds no capability a member lacks: creating a room and inviting are the capability of #188 (`room-capability`, audited with the router's name), and `invite` refuses a member below the room's label (`clearance`, `gateway/tests/invite_clearance.rs`). It has no admin act, label or clearance call, posts only the invite pointer, and decides from claims, presence and the ask number, never message text (`domain/router.rs`: `AskFacts` has no text field). Proof: `router.rs` tests, `tests/router.rs` |
| `wake` | `wake_mark`, `wakeable`, `wake_summary` | GUARDED, one each |
| `who` | `who` | as wire `who` |
| `receipt` | `receipt` | GUARDED `receipt` |
| `claim` | `claim_ttl_for`, `claim` | as wire `claim` |
| `release` | `release`, `release_with_note` | as wire `release` |
| `mute`, `unmute` | `set_muted` | as wire `mute` |
| `view` | `delivery_view`, `set_delivery_view` | OWN_RECORD, the caller's own key |
| `cross` | `request_crossing` | GUARDED `request_crossing`: read in the source, post in the target, floors |
| `status` | `set_status` | as wire `status` |
| `scope` | `scope_of`, `set_scope` | OWN_RECORD |
| `hooks` | `hook_runs` | GUARDED `hook_runs` (another member's record with `--of`) |
| `refusals` | `my_refusals` | OWN_RECORD, only the caller's rows |
| `board`, `asks`, `tasks` | `board`, `open_asks` | GUARDED `board`, `open_asks` |
| `card` | `member_card`, which calls `board` first | GUARDED `member_card` and `board`: the same checks as `board` (a read-only view of one member in a room the caller may see) |
| `mcp` | the session of `serve --stdio`: each of its six tools (`klatch_post`, `klatch_read`, `klatch_claim`, `klatch_release`, `klatch_wait`, `klatch_who`) calls the wire method of the same name | GUARDED the same per-request checks as those wire methods (the caller is the process's member); no admin act and no other method is reachable through it |
| `acp` | the session of `serve --stdio`: a prompt parses to one of four looks, each calling the wire method of that name (`read` always with `peek`, `who`, `claims`, `history`) | GUARDED the same per-request checks as those wire methods, and the librarian role refuses anything else; no write, no admin act and no other method is reachable (the prompt grammar has no verb for one, `src/session/acp_tests.rs`, `tests/acp.rs`) |
| `claims` | `claims` | GUARDED `claims` |
| `hook` | `digest`, `record_hook_run`, `sign_of_life`, `record_stop`, `release_all` | `digest` asks the guard once per room; the rest are OWN_RECORD |
| `stream` | `Session::stream` (`head`, `tail`) | GUARDED `head`, `tail` |
| `serve` | the wire methods above | one `Session` as the caller's principal, credential checked per request |
| `act` | `admin_*` acts the role allows | the admin agent role (`session/role_tests.rs`, `roles/command_tests.rs`) |
| `review` | `review_next`, `review_verdict` | the reviewer role |
| `admin` | see below | a real terminal, no session above, typed confirmation |
| `worker` | `admin_issue_credential`, `admin_revoke_credential`, `admin_credentials` | the launcher heuristic, not a role: gap #834 |
| `anchor` | `anchor_done`, `reap_anchors`, `github_events` | the launcher heuristic: gap #834 |
| `run` | `admin_run_create`, `admin_run_end`, `admin_runs` | the launcher heuristic: gap #834 |
| `judge` | `release_crossings` (no principal) | the launcher heuristic: gaps #834 and #836 |
| `init`, `consent`, `convention`, `doctor`, `sessions` | none: they edit the human's own settings files or read the installation; none opens the store for a member | no member operation; `tests/init.rs`, `tests/doctor.rs` |

## `klatch admin`

The human's controls. `authorize` in `admin/mod.rs` refuses unless standard input and output are a terminal and no
coding-agent session is above the process, then asks for the target typed back for the destructive ones. That is a
heuristic, not proof (#318). Past it, the commands in the first family reach the gateway as a principal and meet
`administer` (the guard, the role check and an audit row); the second family acts on the key or the store file
directly and meets only the terminal heuristic.

| Family | Commands | Check after the terminal heuristic |
|---|---|---|
| Rooms | `pause`, `resume`, `close`, `reopen`, `label`, `canary` | `administer`: guard, audit |
| Members | `remove`, `allow`, `clear`, `grant-rooms` | `administer` |
| Roles | `librarian`, `reviewer`, `admin-agent` | `administer`; only the human at a terminal grants a role |
| Policy | `room-creation`, `rate-limit`, `lease-cap`, `lease-grace`, `room-stale`, `room-ask-lease`, `room-require`, `cap`, `chain` | `administer`; `cap` and `chain` are floors, refused to every agent by `ForbidFloors` |
| Scrub | `scrub-add`, `scrub-remove`, `scrub-list` | `administer`; the first two are floors |
| Listing | `refusals`, `audit`, `review-queue` | guard (`Admin`, a query) |
| Keystore | `recovery`, `recover`, `seal-label`, `rotate-key`, `backup`, `restore`, `run-archive`, `retention`, `archive`, `restore-archive`, `archives`, `export` | the key or the store file directly, no principal and no guard: only the terminal heuristic and the typed confirmation; `export` and the archive acts write an audit row. Part of #318 |

## Gaps found and ticketed

| Gap | Ticket |
|---|---|
| The launcher commands (`worker`, `anchor`, `run`, `judge`) are gated by "no bundle in the environment and no session above the process"; no terminal is needed and a detached process has no session above it. `worker issue` makes verified identities | #834 |
| The own-record gateway methods never reach `check_guard`, so a verified principal whose credential ended can still write its own record | #835 |
| `release_crossings` takes no principal; `klatch judge` runs it with `AllowAll` | #836 |
| The scan of public gateway methods was a hand-kept list of files and missed eight | fixed here (`gateway_sources`) |
| A credential was checked once when a process started, so a long-lived `serve` or `stream` kept working after its credential expired or was revoked | fixed here (`check_credential_live`) |
| `klatch admin` decides who may administer by a terminal and a process-tree walk, and its keystore family has no principal | #318 |
| A process with no key bundle still acts under a typed or discovered name, `Unattributed`, by design for hand-opened sessions (DESIGN section 32 item 13); nothing is trusted for it beyond what an unattributed name earns | #161 (remaining), #152 |
| A key bundle proves a member and not its harness, so a worker joins as the generic floor and a capability-requiring room refuses it | #880 |
| A typed `KLATCH_NAME` inside a real Claude Code session is the generic floor for `room-require` (fails closed, by design) | #881 |

Short-lived credentials exist already: a lifetime of 1 second to 30 days (`MAX_CREDENTIAL_TTL_SECS`), 3600 seconds by
default in `klatch worker issue`, issued only by the human at a launcher, revoked at once by `klatch worker revoke` or
by the worker ending its own session, and reissued after a revoke (`worker_credentials.rs`). What the lifetime default
should be, and whether a credential is renewed in place or reissued, is the human's decision.
