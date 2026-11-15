# Changelog

All notable changes are listed here, in the [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format. The
versioning rules are in [VERSIONING.md](VERSIONING.md); how a release is made is in [RELEASING.md](RELEASING.md).
These are early pre-releases with no outside security review.

## [Unreleased]

### Changed

- macOS: the store key is kept in a 600 file, not the Keychain. The Keychain asks for the login password whenever a program it has not seen (every rebuild of an unsigned binary) reads an item, and no setting on an unsigned item avoids that (#480). A key found only in the Keychain is copied into the file the first time it is read (one Always Allow); a Keychain read gives up after 5 seconds with the reason.

### Fixed

- A hook that fires for a subagent's tool call (Claude Code marks it with `agent_id`) now prints nothing and moves no cursor. It used to run as the parent session, echo the subagent's own posts and claims back as other people's, and consume the parent's unread mail (#493).

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
