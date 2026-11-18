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
  - `check_opens` (`|` to `^` in the open flags, #721): equivalent. `SQLITE_OPEN_READ_ONLY` (0x1) and
    `SQLITE_OPEN_URI` (0x40) share no bit, so `^` and `|` give the same flags; no test can tell them apart.
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

# Second run, after wave 1M (#407 rerun)

Run by hand on 2026-11-18 (`date` at the end: Wed Nov 18 01:28 EST; the run started the evening before) at main commit
2c4eaeb ("Merge pull request #772"), which contains the tests of PRs #766, #768, #769, #770 and #772. Same tool
(cargo-mutants 27.1.0), same config and same eight modules as the first run, through `proto/scripts/mutants`. No
production code was touched and no test was added by this change; the runner gained one option, `MUTANTS_ARGS`
(extra cargo-mutants options), to hold back the looping mutants (see below). **This measures the tests, not the code.**

## Before and after

Before is the first run (commit d4d0bba). After is this run. The total changed for three modules because the code changed
between the two commits (gateway-policy 152 to 158, gateway-claims 126 to 137, archive 136 to 133). Columns: total / caught / missed / timeout / unviable.

| Module | Before | After |
|---|---|---|
| gateway-policy | 152 / 139 / 5 / 0 / 8 | 158 / 148 / 1 / 0 / 9 |
| domain-security | 247 / 234 / 11 / 1 / 1 | 247 / 245 / 0 / 1 / 1 |
| gateway-claims | 126 / 90 / 22 / 0 / 14 | 137 / 122 / 0 / 0 / 15 |
| keys | 234 / 136 / 62 / 0 / 36 | 234 / 143 / 55 / 0 / 36 |
| custody | 115 / 35 / 44 / 2 / 34 | 115 / 73 / 6 / 2 / 34 |
| store-crypto | 106 / 68 / 21 / 4 / 13 | 106 / 75 / 13 / 5 / 13 |
| store-claims | 153 / 103 / 34 / 0 / 16 | 153 / 130 / 7 / 0 / 16 |
| archive | 136 / 55 / 70 / 0 / 11 | 133 / 108 / 14 / 0 / 11 |
| **All** | **1269 / 860 / 269 / 7 / 133** | **1283 / 1044 / 96 / 8 / 135** |

Every row sums (caught + missed + timeout + unviable = total). Caught ratio over viable mutants (caught + timeout over
caught + timeout + missed), before to after: gateway-policy 96% to 99%, domain-security 96% to 100%, gateway-claims
80% to 100%, keys 69% to 72% (40 of its 55 survivors are in code not built here), custody 46% to 93%, store-crypto
77% to 86%, store-claims 75% to 95%, archive 44% to 89%. Over all modules: 1052 of 1148 = 92% (before 867 of 1136 =
76%).

The first run's result files were deleted after it, so "which survivors are now caught" is derived from the tickets
(#716 to #727) and from the module counts, not from a diff of two missed lists. By the tickets, all of #716 to #720,
#722, #724 (except the survivors below), #725 and #726 (except the survivors below) are now caught.

## What still survives (96)

Not ticketed anew because an open ticket already names them, or equivalent or not a security function:

- keys, 55 survivors. 40 are in `keys/linux.rs`, which is listed by cargo-mutants but not built on macOS (the tests
  that would catch them do not run here): this run says nothing about them; they need the same run on Linux. 4 in
  `keys/macos.rs` (the two `ERR_SEC_ITEM_NOT_FOUND` match guards in `get` and `delete`, and `describe` replaced by
  text) and 1 in `keys/bundle.rs:147` (`store.is_none()` guard in `Bundle::read`) are the remaining items of the open
  #723. 10 more are `describe` and `Display::fmt` bodies replaced by empty text or "xyzzy": formatting only, not
  security (listed in the first run too).
- archive, 14 survivors, all in the area of the open #727 (the archive commands and archive integrity):
  `admin/archive.rs` `archive_room` (`>` at line 121 three ways, `+=` at 123), `settle_after_failure` replaced by
  `()`, `finish` (`==` at 149); `store/sqlite/archive.rs` `list_archives` and `archive_files` NotFound guards,
  `archive_before_hooked` (`==` at 314); `store/sqlite/integrity.rs` `open_archives` (replaced by `Ok(vec![])` and
  `==` at 120) and `check_store` (replaced by `Ok(vec![])`, `Ok(vec![String::new()])`, `Ok(vec!["xyzzy".into()])`).
  `check_store` returning an empty list is the important one: a corrupt store would be reported clean. #727 stays
  open; **it is not closed by this rerun**.
- store-crypto, 13 survivors plus 5 timeouts: `check_opens` `|` to `^` is **equivalent** (`SQLITE_OPEN_READ_ONLY` 0x1
  and `SQLITE_OPEN_URI` 0x40 share no bit; first run said the same). The other 12 survivors and the 2 timeouts
  (`%` replaced in the delay) are in `connect.rs` `open_with_timeout` and its busy test: busy-retry timing, not
  security (first run's reasoning). The 3 timeouts in `scrub_and_remove` are discussed below. Every function #721
  names (`verify_copy`, `open_dir`, `convert`, `scrub_and_remove`) now has its mutants caught or timed out.
- custody, 6 survivors: 4 are in `BrokenKeys` (`put` and `delete` replaced by `Ok(())`, `describe` replaced), the
  stand-in used when the key bundle is unusable; `custody.rs:186` `next != key` replaced by `true` in `open_keyed` is
  **equivalent in effect** (when the pending key equals the current key, opening with it fails the same way and the
  original error is returned). `identity.rs:119` `<` to `<=` is a new ticket (#801). 2 timeouts are `next_name`
  replaced by empty text or "xyzzy" (a key name that collides with itself loops until the clock).
- store-claims, 7 survivors: 6 are default bodies of the `ClaimStore` and `CredentialStore` traits that
  `SqliteStore` overrides, dead today (named as a risk for a second Store implementation in the first run and #726);
  `claims.rs:281` `&&` to `||` in `cycle_notices` is a new ticket (#802).
- gateway-policy, 1 survivor: `checks.rs:87` `identifier` replaced by `true`; its security caller is the permission
  mode check in `presence.rs`. New ticket #800.
- domain-security: 0 survivors; 1 timeout (`text.rs:71` `-=` to `/=` in `trim_slashes`, a loop that does not end).
- gateway-claims: 0 survivors.

## The scrub_and_remove loop mutants

The six mutants of `scrub_and_remove` were run last, alone, in a fresh copy, with the runner's `ulimit -f` (1 GiB per
file) and the disk watchdog (polled every 3 s; kill below 30 GB free; 60 GB free was required to start). Free space
stayed at 74 to 78 GB for the whole run: nothing grew without end, so the limit and the loop bound held. Results:
`Ok(())`, `>` to `<` and `-=` to `+=` caught; `>` to `==`, `>` to `>=` and `-=` to `/=` timed out (they loop; a timeout
counts as caught). The disk hazard of the first run (#683) did not repeat.

## Tickets from this run

| # | About |
|---|---|
| 800 | `checks::identifier` replaced by `true` survives (permission mode validation) |
| 801 | `ensure_session_member` suffix growth bound `<` to `<=` |
| 802 | `cycle_notices` holder condition `&&` to `||` |

Open from the first run and still correct: #723 (keys), #727 (archive commands and integrity).

## Limits of this run

- Not run, as before: the other `domain/` files, `transport`, `identity` beyond what `custody` covers, `subscription`,
  `judge`, `session/` beyond `policy.rs`, `harness`, `doctor`, `sync`, `anchor`, and the other gateway submodules.
- `keys/linux.rs` was listed (40 survivors) but not built on macOS, so those figures are not evidence about Linux.
  The real Keychain was not touched (`KLATCH_KEY_DIR` points at a scratch directory).
- The first run's missed list was not kept, so the "now caught" statement rests on counts and tickets, and the total of
  each module differs a little because the code changed between the two commits.
- A caught mutant shows that some test fails, not the right one; an equivalent mutant is counted as a survivor unless
  named above. Timeouts depend on the machine; two modules ran at once on a shared machine.
- Friction: the first attempt failed with all-zero counts because a long scratch path as TMPDIR broke a unix socket
  test (`path must be shorter than SUN_LEN`) in the unmutated baseline, so no mutant ran; the runner reports exit 4 in
  that case. TMPDIR must be short (here `/private/tmp/mra93`). The mutex was held by another agent for a while before
  the run could start.
