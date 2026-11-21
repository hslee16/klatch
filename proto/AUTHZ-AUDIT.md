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
   principals untouched, the breaker not counting it) and the wire test in `session/tests/wire.rs`. The same
   step renews the credential on use (#161): the expiry moves to now plus the idle timeout, never past the hard cap
   from issue, and each renewal is audited as `renew-credential`; proof in `credential_renewal.rs`. The own-record
   methods make the same check without the renewal (`check_own_record_live`, #835), except `release_all`, `record_hook_run`
   for `SessionEnd` and `end_credential`, which the end of a session needs (`AFTER_END` in `zero_trust.rs`).
3. **Roles.** `check_role` in the same function: librarian may only look (`read-only`), reviewer may only review
   (`reviewer`), the facilitator (#190) may only look, create a room, invite, post the invite pointer and send one fixed-form need-info DM to the asker of an open ask (#902; `facilitator`,
   enforced before any guard is asked), only an integrator or the human may take the `merge:main` claim (#865; `integrator`, a plain member, the admin agent and every other role are refused with the next step), admin agent has an allowlist (`admin-scope`, `floors`), members' admin acts need a role on a
   members' gateway, room creation and invite need the capability (`room-capability`). Proof: `admin_role.rs`,
   `librarian.rs`, `reviewer.rs`, `facilitator.rs`, `merge_claim.rs`, `room_capability.rs`, `floors.rs`.
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
`admin_set_admin_agent`, `admin_set_librarian`, `admin_set_reviewer`, `admin_set_rate_limit`, `admin_set_member_rate_limit`, `invite`,
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
| `join` | `join` | `CreateRoom` when new, `Join` | GUARDED `join` (#901): the caller only, no `member` param; `session/tests/facilitator_wire.rs`, `tests/mcp.rs` |
| `asks` | `open_asks` | `Read` without advance | GUARDED `open_asks` (#901): a room above the caller's clearance is refused; `session/tests/facilitator_wire.rs` |
| `invite` | `invite` | `Invite`, then `Post` in `via` | GUARDED `invite`; `room_capability.rs` |
| `grant_room_clearance` | `grant_room_clearance` | `GrantClearance`, then the room's checks and the grantor rules | GUARDED `grant_room_clearance` (#893, #895): `room`, `member` and `label` are params, the grantor is the transport's principal; rules `grantor`, `self-grant`, `top-level`, `agent-cap`, `room-label`, `own-clearance`; audited, a refusal keeps a refusal row; `session/tests/grant.rs`, `gateway/tests/room_grantor.rs` |
| `my_crossings` | `my_crossings` | none (OWN_RECORD, no guard, like `refusals`) | OWN_RECORD `my_crossings` (#961): `limit` is the only param, the rows are the transport's principal's own (the requester of the queue row), no param names another member; ids, pointers, state and a fixed sentence, never text; `session/tests/crossings_wire.rs` |
| `my_role` | `my_role` | none (OWN_RECORD, no guard, like `my_crossings`) | OWN_RECORD `my_role` (#1110): no param is read, the answer is the transport's principal's own role word or null; `session/tests/my_role_wire.rs` |
| `who` | `who` | `Who` | GUARDED `who`; since #181 each member line also carries the observed repository id and the short directory name (never the path, `with_places`), under the same `admit` as the member list itself; since #677 it also carries each member's last reported permission mode (`with_modes`, one read of `member_modes`, display and wake-path choice only, never authority), under the same `admit`; display only, no check reads them (`gateway/tests/member_places.rs`: a reader below the room label gets neither `who` nor the board) |
| `kind` | `set_kind` | none: the caller's own display word | OWN_RECORD `set_kind`; `member_kind.rs`; refused after the credential ends (#835) |
| `status` | `set_status` | `Status` | GUARDED `set_status` |
| `claim` | `claim_ttl_for`, `claim` | `Claim`; the path `merge:main` (any spelling) also needs the integrator role or the human's own name (#865, rule `integrator`, refused before the guard, in `claim_ttl_for` too) | GUARDED `claim`, `claim_ttl_for`; `claims.rs`; `gateway/tests/merge_claim.rs` |
| `scope` | `set_scope` | none: the caller's own scope | OWN_RECORD `set_scope`; refused after the credential ends (#835, `AFTER_END`) |
| `mute` | `set_muted` | `Mute` | GUARDED `set_muted`; `mute.rs` |
| `release` | `release`, `release_with_note` | `Release` | GUARDED `release`, `release_with_note` |
| `claims` | `claims` | `Claims` | GUARDED `claims` |
| `stuck` | `stuck` | GUARDED `stuck`: the same read admit as `open_asks` (`Operation::read`, so a room above the caller's clearance is refused with `clearance`), read only (no cursor, no presence write, no message, no claim), and the events carry ids, member names and numbers, never message text (`domain/stuck.rs`). Proof: `session/tests/stuck.rs`, `gateway/tests/stuck.rs`, the `zero_trust.rs` entry |
| `label_order` | `label_order` | `Admin("label-order")`, any member's, read-only (levels and cap, no rooms or clearances) | GUARDED `label_order`; `session/tests/dm.rs` |
| `scrub_check` | `scrub_check` | `Admin("scrub-check")`, any member's, yes or no only, never a rule name or term, no refusal row | GUARDED `scrub_check`; `session/tests/dm.rs` |
| `prune` | `prune_members` | `Admin("prune")`, every member's, fixed retention | GUARDED `prune_members`; `admin_role.rs` |
| `role_act` | the `admin_*` acts of the act named | `Admin(act)`, then the admin agent allowlist; floors refused | GUARDED `admin_close_room` and the rest; `session/role_tests.rs`, `floors.rs` |
| `review_next` | `review_next` | `ReviewNext`, reviewer role only | GUARDED `review_next`; `reviewer.rs` |
| `review_verdict` | `review_verdict` | `ReviewVerdict`, reviewer role only | GUARDED `review_verdict`; `reviewer.rs`. Releases what it recorded under the reviewer's own name (audit `applied by <reviewer>`) |
| (no wire method) | `release_crossings` | `Admin(release-crossings)`, the human's gateway only | GUARDED `release_crossings`; a reviewer is refused `reviewer`, an admin agent and a plain member on a members' gateway `admin-scope`. Proof: `tests/crossing.rs` (`a_principal_without_the_role_cannot_release`, `a_release_names_who_released_it_in_the_audit`) |

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
| `read` | `read` | as wire `read`; `--count` (#1011) prints only the number of messages a peek would show, through the same `read` peek (never advances, the caller's own record, no new method), proved by `tests/cli.rs` `count_reports_the_unread_total_and_marks_nothing_read` |
| `history` | `history` | as wire `history` |
| `wait` | `wait_poll`, `wait_seen` | GUARDED `wait_poll`, `wait_seen` |
| `invite` | `invite` | as wire `invite` |
| `router` | `join`, `open_asks`, `claims`, `who`, `invite`, repeated by the command, as the caller's own member (#189) | GUARDED each of those calls; the router holds no capability a member lacks: creating a room and inviting are the capability of #188 (`room-capability`, audited with the router's name), and `invite` refuses a member below the room's label (`clearance`, `gateway/tests/invite_clearance.rs`). It has no admin act, label or clearance call, posts only the invite pointer, and decides from claims, presence and the ask number, never message text (`domain/router.rs`: `AskFacts` has no text field); the observed repository (#890) only orders candidates the claims and rules already chose when more qualify than fit, and adds nobody. Proof: `router.rs` tests, `tests/router.rs` |
| `mediator` | `join`, `stuck`, `who`, `post`, `invite`, repeated by the command, as the caller's own member (#271) | GUARDED each of those calls; the mediator only proposes and holds no capability a member lacks: creating a room and inviting are the capability of #188 (`room-capability`, audited with the mediator's name), `invite` refuses a member below the room's label (`clearance`), and `stuck` refuses a room above the caller's own clearance. It has no admin act and calls no release, cancel, grant, clearance or label method; its posts are fixed templates filled with ask numbers and member names, and its decision reads `StuckEvent`, which has no text field (`domain/mediator.rs`), so no wording of an ask steers it. Proof: `mediator.rs` tests, `tests/mediator.rs`, `fixtures/stuck-cases.json` |
| `stuck` (command) | `stuck`, the same read as wire `stuck`, listing one line per event | as wire `stuck` |
| `grant` | `grant_room_clearance` | GUARDED `grant_room_clearance`: the operation `GrantClearance`, then the room's checks (stops, the caller's own clearance for the room), the role (a librarian or reviewer is refused) and the grantor rules (#893): the caller is the room's grantor now (`grantor`), not the grantee (`self-grant`), the label is not the top level (`top-level`), at most the agent cap (`agent-cap`), the room's label (`room-label`) and the caller's own global clearance (`own-clearance`, room grants are not passed on). Audited (`grant-clearance`: grantor, grantee, room, label); a refusal keeps a refusal row. The decision reads the policy table and roles only, never text. The grantor rules and the grant and audit writes run in one store transaction (`atomically`, #897), so a grantor, cap or label change cannot land between the check and the write; a refusal row is written after it ends. No wire method yet (#895). Also the wire method `grant_room_clearance` (#895), the same call. Proof: `gateway/tests/room_grantor.rs`, `tests/room_grantor.rs` |
| `wake` | `wake_mark`, `wakeable`, `wake_summary` | GUARDED, one each |
| `who` | `who` | as wire `who` |
| `receipt` | `receipt` | GUARDED `receipt`; refuses anyone but the sender of the message (`receipt-sender-only`), which is what lets it return per-recipient `shown` and `read` flags (cursors, #675) to that one member; no message text is returned |
| `claim` | `claim_ttl_for`, `claim` | as wire `claim` |
| `release` | `release`, `release_with_note` | as wire `release` |
| `ask` | `post`, `who` | as wire `post` (GUARDED, the room's checks and content rules); `who` only for the reach note, as the `post` command does (#241) |
| `answer` | `open_asks`, `claim_ttl_for`, `claim`, `post`, `release` | GUARDED each of those calls, in that order, as the caller's own member; it is the three commands `claim`, `post` and `release` in one process and holds no capability they lack. A claim held by another member is reported and nothing is posted; the claim is released after a failed post. No new wire method (#241). Proof: `tests/ask_answer.rs` |
| `mute`, `unmute` | `set_muted` | as wire `mute` |
| `view` | `delivery_view`, `set_delivery_view` | OWN_RECORD, the caller's own key; refused after the credential ends (#835) |
| `cross` | `request_crossing` | GUARDED `request_crossing`: read in the source, post in the target, the target must be one the caller has joined (#679), floors |
| `status` | `set_status` | as wire `status` |
| `scope` | `scope_of`, `set_scope` | OWN_RECORD; refused after the credential ends (#835) |
| `role` | `my_role` | OWN_RECORD (#1090; wire method `my_role` #1110, `session/tests/my_role_wire.rs`): the caller's own role as one fixed word, no member parameter, no audit row; refused after the credential ends (`AFTER_END`); `gateway/tests/my_role.rs` |
| `hooks` | `hook_runs` | GUARDED `hook_runs` (another member's record with `--of`) |
| `refusals` | `my_refusals` | OWN_RECORD, only the caller's rows; refused after the credential ends (#835) |
| `crossings` | `my_crossings` | OWN_RECORD (#516), only the caller's rows (the requester of the queue row); no guard, like `refusals`. Returns the ids, the source pointer, the target room, a state (`waiting`, `approved`, `denied`, `released`, `blocked`) and one fixed sentence per state and category (`private-term`, `credential`, `canary`, `gone`, `floor`). Never the text, a redaction, the reviewer's or judge's own reason (it can describe the content) or the name of a scrub rule or term. Proof: `gateway/gateway/tests/crossing.rs` (`a_denied_crossing_says_it_was_denied_and_never_the_reviewers_reason`, `a_blocked_crossing_names_the_category_and_never_the_rule_name_or_term`). Wire method `my_crossings` (#961); refused after the credential ends (#835) |
| `board`, `asks`, `tasks` | `board`, `open_asks` | GUARDED `board`, `open_asks`. Since #864 the board also carries the merge queue (`queue`, `merge_holder`), derived under the same `admit` from the `ready:` and state words in the room and the room's claims; no new operation and no new wire method. The one extra read is the stored role of each distinct sender of a state word (`role_of`, an internal policy read): a state word counts only from an integrator or the human, and a plain member's is ignored, never refused, so posting stays an ordinary `Post`. Proof: `gateway/tests/merge_queue.rs`, `domain/merge_queue/tests.rs` |
| `find` | `history` (paged forward from seq 0, at most 5000 messages) | GUARDED `history`: the same read admit as `history` (a room above the caller's clearance is refused with `clearance`), read only (no cursor, no presence write, no message, no claim). The matching runs in the client on messages `history` already returned, so it sees nothing the caller could not read (`domain/ask_find.rs`). Proof: `tests/find.rs` |
| `card` | `member_card`, which calls `board` first | GUARDED `member_card` and `board`: the same checks as `board` (a read-only view of one member in a room the caller may see) |
| `mcp` | the session of `serve --stdio`: each of its six tools (`klatch_post`, `klatch_read`, `klatch_claim`, `klatch_release`, `klatch_wait`, `klatch_who`) calls the wire method of the same name | GUARDED the same per-request checks as those wire methods (the caller is the process's member); no admin act and no other method is reachable through it |
| `acp` | the session of `serve --stdio`: a prompt parses to one of four looks, each calling the wire method of that name (`read` always with `peek`, `who`, `claims`, `history`) | GUARDED the same per-request checks as those wire methods, and the librarian role refuses anything else; no write, no admin act and no other method is reachable (the prompt grammar has no verb for one, `src/session/acp_tests.rs`, `tests/acp.rs`) |
| `claims` | `claims` | GUARDED `claims` |
| `hook` | `digest`, `record_hook_run`, `sign_of_life`, `record_stop`, `release_all` | `digest` asks the guard once per room and, per room it shows, tells the caller once what became of its own crossings from that room (#679; the caller's own queue rows, pointers and fixed sentences, no text); the rest are OWN_RECORD, refused after the credential ends except `release_all` and `record_hook_run` for `SessionEnd`, which a session end needs (#835) |
| `stream` | `Session::stream` (`head`, `tail`) | GUARDED `head`, `tail` |
| `serve` | the wire methods above | one `Session` as the caller's principal, credential checked per request. With `--ssh-key LABEL` (#107, an ssh forced command) the caller is the member the human mapped LABEL to (`PeerEvidence::SshForcedKey`, `SshKeyIdentity`, verified), an unmapped label does not start (rule `ssh-key-unmapped`, kept in the refusals), a typed name beside it is an error, no environment variable or `SSH_ORIGINAL_COMMAND` is read for identity, and a request that claims an identity is refused (rule `member-fixed`, kept in the refusals under operation `session` with no peer text and counted by the breaker, #1084); the mapping is read again before each batch of requests and on every idle tick, and an open session whose key was unmapped or remapped ends with a refusal (rules `ssh-key-unmapped`, `ssh-key-remapped`, #1083). Proof: `tests/ssh_forced_command.rs`, `tests/ssh_session_recheck.rs`, `session/tests/fixed_member.rs`, `session/tests/recheck.rs`, the bench scenario `two_ssh_forced_command_keys_are_two_members_and_neither_can_claim_to_be_the_other` |
| `act` | `admin_*` acts the role allows | the admin agent role (`session/role_tests.rs`, `roles/command_tests.rs`) |
| `review` | `review_next`, `review_verdict` | the reviewer role |
| `admin` | see below | a real terminal, no session above, typed confirmation |
| `worker` | `admin_issue_credential`, `admin_revoke_credential`, `admin_credentials` | the launcher heuristic, not a role: gap #834 |
| `anchor` | `anchor_done`, `reap_anchors`, `github_events` | the launcher heuristic: gap #834 |
| `run` | `admin_run_create`, `admin_run_end`, `admin_runs` | the launcher heuristic: gap #834 |
| `judge` | `pending_reviews`, `resolve_review`, `record_audit` (via `drain`, which runs the local rules judge: `rules_judge`, `RulesJudge`, `Breaker`; `run` is a resident loop unless `--once`), `release_crossings` as the named principal `judge` (the admin act `release-crossings`: guarded, refusal kept, audit rows say `applied by judge`; no role holder's list names it) | the launcher heuristic: gap #834. A verdict is data: the floors are checked again when a crossing is released and the rules judge only denies or defers. By default no text leaves the machine; with the opt-in `--hosted-judge` and `--hosted-ceiling` (#960) text strictly below the ceiling label goes to an outside service, behind `Narrowed`, `BelowCeiling` and `Breaker` |
| `init`, `consent`, `convention`, `doctor`, `sessions` | none: they edit the human's own settings files or read the installation; none opens the store for a member | no member operation; `tests/init.rs`, `tests/doctor.rs` |

## `klatch admin`

The human's controls. `authorize` in `admin/mod.rs` refuses unless standard input and output are a terminal and no
coding-agent session is above the process, then asks for the target typed back for the destructive ones. That is a
heuristic, not proof (#318). Past it, the commands in the first family reach the gateway as a principal and meet
`administer` (the guard, the role check and an audit row); the second family acts on the key or the store file
directly and meets only the terminal heuristic.

| Family | Commands | Check after the terminal heuristic |
|---|---|---|
| Rooms | `pause`, `resume`, `close`, `reopen`, `label`, `canary`, `canary-retire` | `administer`: guard, audit; `canary-retire` is not in the admin agent's allowlist (`admin-scope`). `label` by the human to `private` or above also plants a canary (#347); an admin agent's `label` plants none |
| Members | `remove`, `allow`, `clear`, `grant-rooms`, `grantor`, `room-clear`, `ssh-key` | `administer`; `ssh-key` (#107, `admin_set_ssh_key`, audited as `ssh-key` with `LABEL/MEMBER=on`) maps an ssh forced-command key label to one member and is a floor, refused to every agent and to the admin agent by `ForbidFloors` and `admin_agent_refusal` (rule `floors`); mapping a label already mapped to another member is refused (rule `ssh-key-mapped`, kept in the refusals), a bad label or member name or a reserved name is `invalid`, and nothing is stored on a refusal. Proof: `gateway/tests/ssh_key.rs`, the `zero_trust` entry `admin_set_ssh_key`, `tests/ssh_forced_command.rs`. `grantor` (name or remove the room's grantor) and `room-clear` (set or revoke a room grant) are floors, refused to every agent and to the admin agent by `ForbidFloors` and `admin_agent_refusal` (#893) |
| Roles | `librarian`, `reviewer`, `facilitator`, `integrator`, `admin-agent` | `administer`; only the human at a terminal grants a role. `integrator` (#865, `admin_set_integrator`, audited as `integrator`) makes the member an ordinary member that alone may take the `merge:main` claim (the human's own name may too); it grants no admin act (`admin-scope` on a members' gateway), no read and no room beyond a member's, and it is not in the admin agent's allowlist. Proof: `gateway/tests/merge_claim.rs`, the `zero_trust.rs` entry. `facilitator` (#190, `admin_set_facilitator`, audited as `facilitator`) gives a member the bounded routing role: read, who, claims, board and open asks at its own clearance, create a room, invite, the exact invite pointer post and (#902, `check_facilitator_dm`, audited as `facilitator-dm` and `create-room`) one need-info DM per open ask to that ask's asker only, in the form `need-info #N: CODE` (CODE from a fixed set of five; the optional free field was dropped in #905, so the message carries no words at all), in a two-member room capped at the lower clearance and needing the room-creation capability; every other operation, every other DM, a second DM for an ask, a DM to anyone but the asker, a DM about a closed ask, every admin act and the room grant are refused with the rule `facilitator` before the guard is asked. Proof: `gateway/tests/facilitator.rs`, `session/facilitator_tests.rs` (a scripted model DMs once; ask text steering it to another member, a second DM or any text after the code is refused) |
| Policy | `room-creation`, `rate-limit`, `rate-limit --member` (act `member-rate-limit`, #972), `lease-cap`, `lease-grace`, `room-stale`, `room-ask-lease`, `room-require`, `cap`, `chain` | `administer`; `cap`, `chain` and `member-rate-limit` are floors, refused to every agent by `ForbidFloors` (a member limit overrides the global one, so an agent must not lift its own) |
| Policy read | `rate-limit --list` (`admin_rate_limits`, action `rate-limit-list`, #1023) | guard (`Admin`, a read), not a floor (`ForbidFloors` allows it), no audit row, outside the admin agent allowlist (`admin-scope`); terminal only. Proof: GUARDED `admin_rate_limits`; `tests/rate_limit.rs` (`listing_the_limits_is_a_read_so_it_is_not_a_floor_and_is_not_audited`) |
| Scrub | `scrub-add`, `scrub-remove`, `scrub-list` | `administer`; the first two are floors |
| Listing | `refusals`, `audit`, `review-queue` | guard (`Admin`, a query) |
| Keystore | `recovery`, `recover`, `seal-label`, `rotate-key`, `backup`, `restore`, `run-archive`, `retention`, `archive`, `restore-archive`, `archives`, `export` | the key or the store file directly, no principal and no guard: only the terminal heuristic and the typed confirmation; `export` and the archive acts write an audit row. Part of #318 |

## `klatch-server daemon` (#102)

One gateway (`SharedGateway`, #140) over a local Unix socket; `SharedGateway::session_backend()` is the `session::Backend`
the session loop runs over. Every request a connection makes is a wire method of the table above, dispatched by the same
`handle` on the owner thread as the connection's principal, so it meets exactly the guards, the role check and the live
credential check of that method. The gateway is built with `for_members()`, as `serve` builds it, so no admin act is
reachable except the roles' own. What is new is who the connection is and what it may do before a frame is read.

| Evidence or path | What it gives | Check |
|---|---|---|
| `PeerEvidence::Process { pid, uid }` from the kernel (`SO_PEERCRED`, `getpeereid` and `LOCAL_PEERPID`) | `unmapped-process`, Unattributed (the empty `EvidenceTable`), the pid never a name | the peer's uid must equal the daemon's, checked in `local_evidence` before a byte is read or written and before the connection's thread exists; unit test with a fake credential (a real second uid is not testable here); `no_network.rs` asserts the order |
| `identify` with `name` (first request only) | the typed name, Unattributed | `LocalIdentity` over `DeclaredName`: the `KLATCH_NAME` rules, reserved and malformed names refused without echo; `tests/daemon.rs` |
| `identify` with `member` and `token` (first request only) | Verified, for a live credential | `Gateway::verify_credential` (token, expiry, revocation, a refusal row without the token), the check `klatch serve --stdio` makes for a bundle; a bad token, another member's name, a name together with a token, or a token without a member ends the connection and never falls back; GUARDED `verify_credential` is unchanged, `Backend::verify_credential` only parses the token text first; `tests/daemon.rs` |
| the socket file and its directory | mode 0600, in a directory owned by the daemon's uid and not group or world writable | `directory_problem`, `path_problem` (the `sun_path` limit), `clear_path` (a live daemon is never stolen from, a non-socket is never removed), `Bound::drop` (removed only if still the daemon's own inode); `tests/daemon.rs` |
| the connection and frame limits | 1 MiB frames, a first-frame timeout, a connection cap with a refusal frame | `tests/daemon.rs`; bounded memory: a line over the cap is never stored, one thread and one bounded output queue per connection, at most the cap |

No new gateway method: `Gateway::verify_credential` already exists (GUARDED list unchanged, zero-trust lists unchanged), and
`session::Backend` is a trait over calls the session loop already made. The daemon opens the store through
`klatch::member::spawn_shared_gateway` (same environment variables, same custody as `klatch mcp --stdio`). Label keys and
the harness are per connection (#1096), neither a new method nor a new guard: for each request the owner reads from the
keystore the keys of the sealed labels that request's principal is cleared to (`sealed_labels_within`, the rule
`klatch serve --stdio` uses; no other key, none kept after the request, a failure to load is no key), and a connection that
never sent `identify` whose kernel-reported pid is inside a verified Claude Code session (`peer_runs_claude_code`, the walk a
command makes) joins as Claude Code, every other connection as the generic floor (typed name, #881; credential, #880).
Tests: `shared_session_tests` (keys asked only within clearance, none without a source, harness per connection) and
`tests/daemon.rs` (a sealed label over the socket, with and without a key in the keystore).

## `klatch-server http` (#1059)

The klatch wire over HTTP and server-sent events, on loopback only. A stream (`GET /v1/stream`) runs the session loop of
`klatch serve --stdio` over the same `SharedGateway` as the daemon; `POST /v1/frames` writes frames into a session. Every
frame is a wire method of the table above, run as the session's principal on the owner thread, so it meets the same guards,
role checks and live credential check. No new gateway method and no new `PeerEvidence` variant: the evidence is the existing
`PeerEvidence::Credential` (`Bearer` and the evidence table of item 53 stay for a later transport).

| Evidence or path | What it gives | Check |
|---|---|---|
| `Authorization: Bearer TOKEN` with `X-Klatch-Member` on `GET /v1/stream` and on `POST /v1/frames` | Verified, for a live credential | `Backend::verify_credential` (token, expiry, revocation, a refusal row without the token), the check the daemon's `identify` makes, then `LocalIdentity` over `PeerEvidence::Credential`; every other case (missing header, wrong or other member's token, wrong scheme, a typed name) is the same `401 unauthorized` and nothing falls back to a weaker name; there is no unattributed tier over HTTP; the token is never logged or echoed; `tests/http_wire.rs` |
| `X-Klatch-Session` on `POST /v1/frames` | the open session of the same member | a random 128-bit id; another member's session, an ended one and a malformed id are all `404`; the credential is checked before the session is looked up and before the body is read; `tests/http_wire.rs` |
| the listener | `127.0.0.1` or `::1` only | `loopback::bind_loopback` refuses every other address, named, before the store is opened; the scan test `tests/no_network.rs` (one `TcpListener::bind`; `wire_http.rs` names no socket type) |
| the limits | 16 KiB headers, 1 MiB bodies (413 before the body is read), a 5 second request deadline, a connection cap with `503`, a bounded inbox per session (`429`), the session's bounded output queue and stall limit (#147) | `tests/http_wire.rs`; a client that goes away ends its session at once |

## `klatch-server http` MCP route (#1060)

`POST /mcp` on the same listener: the nine MCP tools (the `McpServer` adapter over `Session::call`, so each is a wire method
of the table above, run as the session's principal with the same guards). No new gateway method and no new evidence.

| Evidence or path | What it gives | Check |
|---|---|---|
| `Authorization: Bearer TOKEN` with `X-Klatch-Member` on every `POST /mcp` | Verified, for a live credential | the same `authenticate` as the wire routes (`verify_credential`, then `LocalIdentity` over `PeerEvidence::Credential`), on every request and with no session to outlive it; every failure is the same `401 unauthorized`; the caller is never an argument (`member` and `sender` are ignored); `tests/mcp_http.rs` (matrix, revoked credential, identity of a post) |
| `Origin` | a page on another origin is `403` | after the credential; only `http` and `https` on `127.0.0.1`, `localhost` or `[::1]` pass; `tests/mcp_http.rs` |
| `initialize` | tools only; never the channel capability | `with_channel` is not called over HTTP; `tests/mcp_http.rs` |
| the limits | the listener's: headers, bodies (413 before read), deadline, connection cap, loopback only | `tests/mcp_http.rs` |

## Gaps found and ticketed

| Gap | Ticket |
|---|---|
| A `member-fixed` refusal on an ssh session was not kept in the refusals or counted by the breaker | fixed in #1084 (`note_member_fixed`, an OWN_RECORD method; the rule is a breaker safety rule) |
| The launcher commands (`worker`, `anchor`, `run`, `judge`) are gated by "no bundle in the environment and no session above the process"; no terminal is needed and a detached process has no session above it. `worker issue` makes verified identities | #834 |
| The own-record gateway methods never reached `check_guard`, so a verified principal whose credential ended could still write its own record | fixed in #835 (`check_own_record_live`); `release_all`, `record_hook_run` for `SessionEnd` and `end_credential` stay allowed after the end, each with its reason in `AFTER_END` |
| `set_status` in the session-end hook is a guarded call, so an idle-expired credential cannot mark the member `done` at session end (claims are still freed) | #1027 |
| `release_crossings` took no principal; `klatch judge` ran it with `AllowAll` | fixed here (#836): it takes a principal, is the admin act `release-crossings`, and each crossing audit row says who applied it; the launcher heuristic that gates the `judge` command remains #834 |
| The scan of public gateway methods was a hand-kept list of files and missed eight | fixed here (`gateway_sources`) |
| A credential was checked once when a process started, so a long-lived `serve` or `stream` kept working after its credential expired or was revoked | fixed here (`check_credential_live`) |
| `klatch admin` decides who may administer by a terminal and a process-tree walk, and its keystore family has no principal | #318 |
| The daemon (#102) carried no sealed-label keys and no harness evidence per connection | fixed in #1096 (per-request keys within clearance; harness from the peer pid for an unidentified connection). A connection that identifies still joins as the generic floor (#880, #881) |
| A wire `join` over `klatch serve --stdio` was judged as the generic floor; only the CLI commands passed the harness | fixed in #1115 (the process-tree evidence is passed to the session; ssh sessions and mcp stay the floor) |
| A process with no key bundle still acts under a typed or discovered name, `Unattributed`, by design for hand-opened sessions (DESIGN section 32 item 13); nothing is trusted for it beyond what an unattributed name earns | #161 (remaining), #152 |
| A key bundle proves a member and not its harness, so a worker joins as the generic floor and a capability-requiring room refuses it | #880 |
| A typed `KLATCH_NAME` inside a real Claude Code session is the generic floor for `room-require` (fails closed, by design) | #881 |
| A room grant raises clearance for one room's checks only and never hands out a label key (`sealed_labels_within` reads the global clearance), by decision | fixed in #896 (intended, option A): `grant_room_clearance` refuses with rule `sealed-key` when the room's label is sealed and the grantee's global clearance does not reach it, so the failure shows at grant time. The human's `admin_set_room_clearance` is not checked (a floor act) |
| The facilitator's need-info DM field of #902 (a free field checked only against lobby text) is removed (#905, #997); the DM is a code only, so nothing derived from other members' text can ride in it | fixed in #905 |
| The member attributes (stable id, repository, directory; #181) add no wire method and no command: the host records them for a member that is already acting, and no check reads them. The repository value is an observation (a directory can name another repository's git directory), so it grants no clearance, room or access; the lowest clearance for a first-seen repository is pinned by `a_first_seen_repository_session_holds_the_lowest_clearance_and_cannot_read_above_it` (#191). `who`, the board and the router now show or read them: see the `who` row (guarded by the existing `Who` and board `Read` admit, repository id and short directory name only) and the `router` row (the repository only orders members that claims and rules already chose) | #181 |

Short-lived credentials (#161): an issued credential lives for an idle timeout (`--ttl`, 3600 seconds by default) that
each successful request pushes out again, up to a hard cap from issue (`--max-life`, 24 hours by default, at most 30
days, `MAX_CREDENTIAL_TTL_SECS`) that renewal never passes. They are issued only by the human at a launcher, revoked at
once by `klatch worker revoke` or by the worker ending its own session, and rotated by revoke and reissue
(`worker_credentials.rs`, `credential_renewal.rs`). A credential issued before schema 31 is never renewed.
