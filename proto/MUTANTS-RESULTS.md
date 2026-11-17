# Mutation testing of the security modules (#407)

Run by hand on 2026-11-17 (`date` at the start: Tue Nov 17 12:15 EST), as DESIGN item 66 and decision #402 prescribe:
before a release, not in CI, not nightly. A surviving mutant in a security relevant function became a GitHub ticket
(#716 to #727). **This measures how good the tests are. It is not a claim that the code is correct, and a module with
few survivors is not thereby secure.**

## Setup

- Tool: cargo-mutants 27.1.0, run through `proto/scripts/mutants` with `proto/klatch/.cargo/mutants.toml`.
- Code: commit d4d0bba (main at the time, "Merge pull request #696"), unchanged. No production code was touched.
- Host: macOS (aarch64), so code under `cfg(target_os = "linux")` was not built and says nothing (see below).
- Bounds: test timeout 3 times the unmutated run (floor 60 s; 220 to 250 s in practice), build timeout 3 times.
  `ulimit -f` 1 GiB per process (added after the disk incident below). Two modules ran at once in separate copies of
  the repository. The whole package test suite (60 test binaries, about 950 unit tests plus the integration tests)
  runs for each mutant.
- Excluded from mutation (config): test code (`**/tests/**`, `*_tests.rs`), `kani_proofs.rs`, `shuttle_tests.rs`,
  `store/conformance/**`, `gateway/test_support.rs`, `store/memory_spike.rs`. They are test or proof harness, not what
  is measured. No mutant was excluded for being logging or formatting; those are in the counts and called out below.
- Result files (kept outside the repository, deleted after this run): `mutants.out/` per module with `missed.txt`,
  `caught.txt`, `unviable.txt`, `timeout.txt`, `outcomes.json` and one diff per mutant.

## Results

Caught means a test failed. Missed means every test passed (the survivor). Unviable means the changed code did not
compile (no information). Timeout means the tests ran past the limit; cargo-mutants counts it as caught, it is listed
apart here (usually a mutant that loops forever, which a test run does notice, but only through the clock).

| Module | Files | Total | Caught | Missed | Timeout | Unviable |
|---|---|---:|---:|---:|---:|---:|
| gateway-policy | gateway/guard.rs, gateway/{policy,checks,scrub,tripwire,crossing}.rs, session/policy.rs | 152 | 139 | 5 | 0 | 8 |
| domain-security | domain/{scope,text,labels,toolguard,credentials,claim_age}.rs | 247 | 234 | 11 | 1 | 1 |
| gateway-claims | gateway/{claims,lease,roles}.rs, gateway/sys_lease.rs, roles.rs, scope.rs | 126 | 90 | 22 | 0 | 14 |
| keys | keys/*.rs | 234 | 136 | 62 | 0 | 36 |
| custody | custody.rs, identity.rs | 115 | 35 | 44 | 2 | 34 |
| store-crypto | store/encrypt.rs, store/sqlite/{seal,connect}.rs | 106 | 68 | 21 | 4 | 13 |
| store-claims | store/sqlite/{claims,leases,credentials}.rs, store/sqlite/api/{claims,credentials}.rs, store/area/{claims,credentials}.rs | 153 | 103 | 34 | 0 | 16 |
| archive | admin/{archive,keys}.rs, store/sqlite/{archive,backup,integrity}.rs | 136 | 55 | 70 | 0 | 11 |
| **All** | | **1269** | **860** | **269** | **7** | **133** |

Total mutants equals the number cargo-mutants listed for these files (1269). Caught plus missed plus timeout plus
unviable equals the total for every row. Caught ratio over viable mutants (caught + timeout over caught + timeout +
missed): gateway-policy 96%, domain-security 96%, gateway-claims 80%, keys 69%, custody 46%, store-crypto 77%,
store-claims 75%, archive 44%.

Two modules (domain-security, store-crypto) were interrupted once (see friction) and finished with `--iterate`; their
rows merge the two runs, the later result winning for a mutant seen twice.

## What the survivors say

- The gateway policy and domain checks (refusal rules, labels, clearance, scrub, credential detection) are well
  tested: 96% and 96%. What survives there is boundaries and the false-positive side of detection (#716, #718, #719,
  #720) and one canary hash (#717).
- The weakest areas are the ones one layer above the store: the admin command layer for archives and keys (#727),
  custody and identity (#724), and the encryption conversion path (#721). Whole-function replacements by `Ok(())`
  survive in `verify_copy`, `restore_key`-adjacent code, `archive_command` and the key commands: the tests exercise the
  store code below them, not the commands.
- Credential expiry (`is_live`, `expires > now`) and the claim blocker graph have unpinned conditions (#726).
- Not ticketed, with reason:
  - `keys/linux.rs`: 22 survivors in code that is not compiled on macOS. The run says nothing about it; it needs the
    same run on Linux.
  - `connect.rs` busy-retry arithmetic (12 survivors in `open_with_timeout`): timing code, not security.
  - `describe` and `Display::fmt` bodies replaced by empty text (several, not counted): pure formatting of messages.
  - Trait default bodies in `store/area/*.rs` that SqliteStore overrides: dead today (listed in #726 as a risk for a
    second Store implementation).
  - Equivalent mutants were not separated from real survivors by hand except where a ticket says "close to
    equivalent" (#717, #720). Some other survivors are probably equivalent too; the counts do not remove them.

## Not run, and limits

- Everything listed above was run. Not covered by this run: the other `domain/` files (hook, board, asks, tasks,
  convention, receipt, parked, model, error), `gateway/guard`-adjacent files outside the module table (`transport`,
  `identity`, `subscription`, `listener`, `judge`), `session/` beyond `policy.rs`, `harness/`, `doctor/`, `sync.rs`,
  `anchor.rs`, `codex.rs`, and the other gateway submodules (`admin`, `workers`, `rooms`, `runs`, `presence`,
  `board`, `reading`, `anchors`, `github_anchor`). They are less security central; they were chosen out, not run.
- Linux-only code (`keys/linux.rs`) not exercised, as above; macOS-only code (`keys/macos.rs`) ran, but the real
  Keychain was not touched (tests use temp homes and KLATCH_KEY_DIR; the runner points KLATCH_KEY_DIR at a scratch
  directory).
- A test that fails for an unrelated reason (a timeout of a slow test on a busy machine, a flaky test) counts a mutant
  as caught. The machine was shared with other jobs; timeouts here were 3 times the unmutated time.
- One interleaving risk: between modules a cleanup of the run's own temporary directory ran while the other module's
  copy was possibly mid-test; a test failing from that would be counted caught. It could affect only a mutant that was
  running at that moment.
- A caught mutant shows that some test fails, not that the right test does, and a missed mutant in a function whose
  behaviour is unspecified is not a bug.

## Friction found while measuring (also fixed in the runner)

1. The runner copies the repository with rsync, which restores old modification times. cargo then kept the build of
   the last mutant of the previous module, and the baseline of the next module failed on a stale build. The runner now
   touches the copied sources after each copy.
2. Disk. Free space fell from 60 GB to 23 GB in 2 minutes (about 3 GB per 10 s) while a store-crypto mutant ran (the
   loops in `scrub_and_remove` are the likely cause, or another mutant of that module; the run was killed by the disk
   watchdog and the machine briefly had 0 free). The watchdog polled every 20 s, too slow for that rate. The runner
   now sets `ulimit -f 1048576` so a runaway writer fails at 1 GiB, and the driver polls every 3 s. The disk recovered
   once the processes were killed and their temporary files deleted (the scratch copies and TMPDIR were removed at
   the end). Another agent's benchmark was running on the machine at the same time, so the cause is not proven to be
   this run alone. This is the hazard of #683 again, now with a mutant as its source.
3. `--iterate` was added (`MUTANTS_ITERATE=1`) so a run that was interrupted can continue.
4. A full pass at two modules in parallel took about 3 hours 35 minutes of wall time (12:16 to 15:50, including the
   interruption and restart).

## Tickets

| # | About |
|---|---|
| 716 | crossing `message_label` |
| 717 | tripwire `canary_token` |
| 718 | `credential_kind`, `json_web_token` |
| 719 | toolguard `is_path_claim`, `clip`, `describe`, `judge_tool` |
| 720 | `MAX_PAYLOAD_BYTES`, `LabelChain::default_cap` |
| 721 | store encryption: `verify_copy`, `scrub_and_remove`, `open_dir`, `convert`, `check_opens` |
| 722 | `SqliteStore::init` version gate |
| 723 | keys: bundle member validation, `label_key_name`, `NotFound` guards |
| 724 | custody and identity |
| 725 | gateway roles, scope, lease |
| 726 | claim blockers, credential expiry, cycle notices, dead trait defaults |
| 727 | admin archive and key commands, archive integrity |

No tests were added in this change: the survivors are reported first and each ticket names the missing test, so the
tests can be written red first against the mutant.
