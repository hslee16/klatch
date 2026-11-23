Code health K12: klatch integration tests, proto/klatch/tests (#1453; test code only, no production change; 2026-11-23).
Behaviour-preserving cleanup of the integration tests named by the CodeScene inventory (epic #1441). No assertion was weakened, loosened or deleted: assertion runs became `assert_has` / `assert_lacks` (new in `common/mod.rs`, they fail naming the missing or unexpected text and show the whole text), repeated setup became helpers, long functions were split. Nothing was red before (3379 passed, 0 failed, 7 ignored for `cargo test --release -p klatch --tests`) and the same test names run after (listing compared by binary and name; only `admin_proof` tests moved binary, see below).

Files moved: `admin_proof.rs` (1216 lines) became `admin_proof.rs` (refusals, proof, lock-out), `admin_proof_passphrase.rs` (life cycle, recovery, prompts) and `admin_proof_terminal.rs` (shared or foreign terminals, signals), over `common/admin.rs` (tables, `detached`, `recovery_code_of`, `repeated_on_one_pty`, `terminal_after_signal`). `detached` now uses the existing `Detach` trait, so the admin files hold no `unsafe`; the stale `admin_proof.rs` entry in the unsafe allow list of hygiene.rs was removed. `models.rs` moved its role table and attempt list to `models/roles.rs` (it was over the 600 line cap).

Other changes: `common/pty.rs` (a `Layout` enum and `Setup` replace the `layout: u8` and flag arguments of `run_with`; `Typist`, `pump`, `wire_stdio` split out; public functions unchanged), `no_telemetry.rs` and `hygiene.rs` (shared feature-table parser, named predicates), `properties.rs` (generators and checks split out, draw order kept), `auto_checkpoint.rs` (a `Session` value instead of the five-argument `tool`), `client_scenario/mod.rs` (`code_of`, grouped checks; it is also included by the klatch-server tests).

| File | Before | After |
| --- | --- | --- |
| admin_proof.rs (and the two new files) | 7.21 | 10.0, 10.0, 10.0 |
| no_telemetry.rs | 7.32 | 9.68 |
| hygiene.rs | 7.78 | 10.0 |
| properties.rs | 8.05 | 10.0 |
| auto_checkpoint.rs | 8.81 | 9.68 |
| common/pty.rs | 8.81 | 10.0 |
| client_scenario/mod.rs | 9.09 | 10.0 |
| caller.rs | 9.24 | 10.0 |
| authz_audit.rs | 9.38 | 9.68 |
| client_methods.rs, crypto.rs, init.rs, late_joiner_briefing.rs, toolguard.rs | 9.38 | 10.0 |
| models.rs (and models/roles.rs) | 9.38 | 10.0, 10.0 |
| shared_forward.rs | 9.38 | 10.0 |
| dm_help.rs | 9.53 | 10.0 |
| ssh_session_recheck.rs | 9.55 | 10.0 |
| priority_digest.rs, scope_mismatch.rs | 9.68 | 10.0 |
| common/mod.rs, linux_headless.rs, ssh_forced_command.rs, crypto-vectors.py | 9.68 | 9.68 (common/mod.rs only gained the two assert helpers; the rest untouched) |

Left alone: the String Heavy Function Arguments findings (test helpers taking several string slices by design), `no_telemetry.rs` and `auto_checkpoint.rs` for the same reason.
