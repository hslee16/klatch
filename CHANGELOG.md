# Changelog

All notable changes are listed here, in the [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format. The
versioning rules are in [VERSIONING.md](VERSIONING.md); how a release is made is in [RELEASING.md](RELEASING.md).
These are early pre-releases with no outside security review.

## [Unreleased]

## [0.1.0-dev.2] - 2026-11-15

### Added

- `klatch convention [--name N]` prints the convention for any agent, with this binary's own path and `KLATCH_NAME=<name>` in front of every command, so a parent can paste it into a subagent's brief (#491). It needs no store, key or name.

- Floors cannot be edited by an agent: a gateway guard (`ForbidFloors`) refuses setting the agent cap, the label order and the scrub list on every gateway an agent can reach (R29, #49). Only `klatch admin` from your own terminal changes them.
- A read-only librarian role: `klatch admin librarian <member>` (`--off` to remove). A librarian can read without moving its cursor, see who, claims, the board and the audit and refusal records, and is refused everything else with the rule `read-only` (DESIGN section 32 item 33, #52).
- An admin agent role: `klatch admin admin-agent <member>` (`--off` to remove), on the same one-role-per-member mechanism as the librarian. It may do the non-floor admin acts and is refused (rules `floors`, `admin-scope`) and audited on anything else; its acts are audited with its name (DESIGN section 32 item 35, #50).
- Review queue and the `Judge` seam (#48): a `review_queue` table (schema 23) of pointers to messages, a one-function `Judge` trait scoped to channel safety, a `NullJudge` that decides nothing, and `klatch judge run --once`, which runs the queue past the judge once. With no judge every function works and the queue only grows. Nothing enqueues yet and a verdict is recorded, not applied (#497).
- `klatch doctor`: a read-only health check of the binary, the store and its key, the hooks (does each point at a binary that exists, and at this one?), the session and the harness registry, with the reason and the fix for each problem; exits 1 on a failure. It creates nothing, never prints a key, and gives up on a stuck keystore after 5 seconds.
- Retention and archive: `klatch admin retention`, `klatch admin archive` and `klatch admin restore-archive`. Old messages move into an encrypted archive file and leave a tombstone (the `seq` stays), and come back on restore. Nothing is archived unless a retention is set.
- `klatch admin export <room> --json`: the transcript as one JSON object per line with every field, regenerated from the store each time (still audited and typed back).
- `klatch sessions`: the live coding-agent sessions on this machine, from the harness registry (pid, name, directory, session id); `--all` adds the records that are not live and why. Read only; never opens a key file.
- A claim may last at most 24 hours (DESIGN D5); a longer request is refused with the limit. Claim again to renew.
- `klatch init --uninstall`: removes every hook klatch installed and nothing else, with a diff and a question per scope.
- Static Linux builds (musl, x86_64 and aarch64): no glibc or other system library needed (tested in containers on Alpine 3.20, Debian 11 and Ubuntu 22.04, arm64 only). The installer chooses them when
  glibc is older than 2.34 or the system is not glibc; the dynamic builds need glibc 2.34 or later.
- Platform wheels on PyPI for macOS (arm64, x86_64) and Linux (x86_64, aarch64): `pipx install klatch` and
  `uvx klatch` install the `klatch` command and the Python client. The Linux wheels are the static builds, so one wheel
  serves glibc and musl systems; other platforms get the client only.
- UNINSTALL.md: what klatch keeps (hooks, binary, rooms, key), how to remove each, and how an upgrade behaves.
- `INTEGRATION.md`, a guide for harnesses, TUIs and agents (spawn `klatch stream` or `serve --stdio`, authenticate, read events, post, resume) with complete Python and TypeScript examples in `clients/examples/`, run by `clients/recipes/check-integration-examples.sh` in the gate and in CI (#101).
- Transport seam for later network transports: Connector and Listener traits, new PeerEvidence kinds (SSH key and forced-command member, pid and uid, certificate subject, bearer token), an EvidenceTable identity and `serve_connection`; unmapped peers are unattributed (#105). No socket transport yet.
- Parked state (#305): a post `parked: <why>, back by 3d` or a Stop hook that sees background tasks or crons keeps a member listed, its claims and asks held, and pruning off it until the return time; any post or hook ends it (schema 25).
- Long tasks (#306): pruning spares a member that holds a live claim (its read cursor and sessions stay), and an ask may carry a deadline, `seeking: ... (by 2026-11-20)`, shown as `due in 3d` or `overdue 2h` and used instead of the 60 second stale threshold.
- Durable tasks (#307): `working:`, `checkpoint:`, `handoff: <task> to <member>` and `finished:` give a derived task view on the board and in `klatch tasks`; a task whose owner is gone stays visible as orphaned with its last checkpoint, and an owner with an open task is spared by pruning for a week.
- A liveness probe in the harness module (`klatch::harness::probe`): from process identity, zombie or stopped state, the registry turn state and progress (transcript size and time, process-tree CPU between two samples, live children, background-task output) it says gone, waiting on a human, working, waiting on a tool, stuck or unknown, with the evidence. An unreadable layer is unknown, never gone. Reads file metadata only (#309).
- An ignored test, `liveness_real`, runs the liveness probe against the real Claude Code sessions on a machine and prints verdicts, evidence and cost; RESEARCH-NOTES section 46 records the first run (#310).
- `klatch doctor` now tries each liveness probe layer on the live sessions and says which are readable on this machine, warning when the registry status or the transcript layout stopped being readable for every session (a Claude Code change). An unreadable layer is unknown, never a dead session (#311).
- The Linux Secret Service backend is now run against a real gnome-keyring in containers (`proto/linux-keys/run.sh`, a manual workflow, never on pull requests) and covered by tests: round trip, replace, delete, wrong size, unsafe names, no secret on any command line, headless fallbacks (#381 #382 #383 #384 #385).
- `klatch admin archives` lists the archive files with rooms, message counts, ages and size, opening each with the store key and checking it for damage (exit status 1 if any is unusable); `klatch doctor` checks them too and warns about a file left by an interrupted `admin archive`. Archives are still never pruned (#487, #568).
- Clearer identity messages: a missing member name says to set `KLATCH_NAME` or pass `--discover`; `join` by discovery prints the name to set next; an empty `read` explains that earlier reads and processes sharing a name consume messages (#495, #500).
- The review queue is now real (#497): `klatch cross ROOM SEQ --to ROOM` queues a crossing into a lower-labelled room, an allow or redact verdict from the judge or the reviewer releases the text (floors checked again), a deny releases nothing, and `klatch admin review-queue` lists the queue without text. Store schema is now 24.
- Changelog fragments: one file per change in `changelog.d/`, checked by the gate and CI, so parallel pull requests no longer collide on CHANGELOG.md; CONTRIBUTING.md says how to reserve a DESIGN item number (#502).
- A member path for roles (#506, #509, #507): `klatch act <act>` and the wire method `role_act` let the admin agent call close, reopen, remove, allow, clear, label, audit and refusals as itself; `klatch review next|verdict` and the wire methods `review_next` and `review_verdict` let the reviewer review. No floor act has a form there, `klatch admin` stays human-only, and a member without the role is refused (`admin-scope`, `floors`, `reviewer`).
- A quarantined reviewer role (#51): `klatch admin reviewer <member>` (`--off` to remove). A reviewer can read queued review items and record allow, redact or deny on them (final, audited without the text), and every other operation is refused before the guard runs. A member holds one role at a time.
- Claims can be anchored to a GitHub pull request or issue (`gh:pr:OWNER/REPO/N`, `gh:issue:OWNER/REPO/N`; `klatch anchor github` reads `gh api` output or a webhook payload and frees the claims of merged or closed items), to a worktree (`wt:NAME`; `klatch anchor reap --worktrees DIR`) and to a session (`session:ID`, freed by `reap` once the session is marked done); klatch makes no network call (#57, #58).

### Changed

- macOS: the store key is kept in a 600 file, not the Keychain. The Keychain asks for the login password whenever a program it has not seen (every rebuild of an unsigned binary) reads an item, and no setting on an unsigned item avoids that (#480). A key found only in the Keychain is copied into the file the first time it is read (one Always Allow); a Keychain read gives up after 5 seconds with the reason.
- Claims follow their holder's liveness (DESIGN section 29, #304): when a member asks about claims (a claim attempt, `claims`, `who`), every other holder is probed. A holder that is working, waiting on a tool or waiting on a human keeps its claims even past their time to live (an expired row nobody took is put back); a holder that is gone loses them a grace period later (5 minutes), a stuck one at once, and one busy with no progress for the cap (4 hours; policy key `lease:cap` in seconds raises it) at once. A holder the probe cannot read, or with no session (a bot), keeps only its explicit time to live.
- A claim no longer lapses while its owner is working: any activity by the owner in the room (a hook, a post, a read) renews each of its live claims back to the ttl that claim was made with, counted from that moment (a `--ttl 60` claim stays 60 s from the last activity, a 3600 s claim stays 3600 s). It never shortens a lease, never revives a lapsed claim, and leaves `ask:` leases alone; the 24 h maximum still holds (#494, schema 26 stores the ttl; claims made before it are not renewed).
- Every job in the release workflows now has a time limit (builds 30 minutes, publishing 15 to 20, the binaries build 45), and `release/check-timeouts.sh` fails the gate and CI if one is missing.

### Fixed

- A hook that fires for a subagent's tool call (Claude Code marks it with `agent_id`) now prints nothing and moves no cursor. It used to run as the parent session, echo the subagent's own posts and claims back as other people's, and consume the parent's unread mail (#493).
- A hook digest is now kept within 8000 bytes (counted after JSON escaping) so it stays under Claude Code's hook output cap, past which the model sees only a 2 KB preview. What does not fit is counted in a line that says to run `klatch read ROOM --peek` (#549).
- The wake pointer now carries the unread count and the newest seq (`3 unread in room r, newest #41`), because Claude Code silently drops a nudge whose text equals a recent one (#550).
- On Linux a session bus address that points nowhere (an inherited one over ssh or cron) no longer makes every command fail: klatch falls back to the key file with the usual warning. Every `secret-tool` call is also bounded to 10 seconds and has no terminal, and a failure to reach the keyring is an error, no longer a missing key (#560).

## [0.1.0-dev.1] - 2026-11-14

The first release published through the release workflow, to PyPI, npm and crates.io together, with binaries for macOS
(arm64, x86_64) and Linux (x86_64, aarch64).

### Added

- Rooms with a durable log, a board of asks and offers, statuses, direct messages and mute.
- Claims on paths with expiry, conflict detection by scope, and an optional PreToolUse hook that blocks an edit of a
  path another member claimed.
- Claude Code hooks installed by `klatch init` (with a diff and a yes per scope), an idle-session wake watcher, and
  `klatch invite`.
- Encryption at rest (SQLCipher, key in the OS keystore or a 0600 key file, printed recovery code, no escrow) and
  per-label sealed bodies.
- Worker credentials: key bundles written by a launcher, expiring and revocable, so a worker's messages are verified.
- Factory features: typed handoffs, anchored claims, run-scoped rooms with encrypted archives.
- The wire protocol (version 1) and clients in Python, TypeScript and Rust (the `client` feature) with one
  conformance scenario each.
- Release files with provenance attestations, a CycloneDX SBOM per target, signed checksums and third-party licence
  notices; an installer script, a Homebrew tap and cargo-binstall metadata.

### Changed

- The ten workspace crates are one package, `klatch`, with the `cli` (default) and `client` features.
- `klatch --version` also prints the protocol and store schema versions.

## [0.1.0-alpha.0] - 2026-11-14

crates.io only: the first publish of the `klatch` crate, made by hand to hold the name. Superseded by 0.1.0-dev.1.
