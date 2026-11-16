# Contributing

## CI is kept light, on purpose

The human's rule (2026-11-14): **keep CI lightweight for now.** Every pull request runs one job on Linux and one on
macOS, each about two minutes. A heavier check is added to CI only when it earns its place, and the default for a new
heavy check is a **manual** workflow (`workflow_dispatch`) that is run by hand, not a trigger on pull requests.

**Runs on every pull request** (`.github/workflows/ci.yml`): `cargo fmt --check`, clippy with warnings as errors, build,
the whole test suite, the `client` and bare feature builds, `cargo publish --dry-run`, the Python and TypeScript
client suites, the launcher demo, the README quickstart, the installer tests, the release version agreement check
(`release/check-versions.sh`), the test-clock test, shellcheck, actionlint, the licence notices check and, on Linux,
cargo-deny.

**Not in CI (run by hand when you need them):**

| What | How |
|---|---|
| the bench (all scenarios) | `cargo test -p klatch-bench -- --ignored`, or `cargo run -p klatch-bench` |
| timing budgets | `klatch-bench --budgets proto/klatch-bench/baseline.json` |
| Linux portability (glibc needs, musl static builds) | Actions, Linux portability, Run workflow; switch it on for pull requests at v1.0 |
| Linux key custody (the Secret Service against a real gnome-keyring, the headless fallbacks, key latency) | `proto/linux-keys/run.sh` (Docker; `--bench` adds latency), or Actions, Linux keys, Run workflow; run it after changing `proto/klatch/src/keys`, `custody.rs` or `proto/linux-keys` |
| release dry run (every build a release does, nothing published) | Actions, Release dry run, Run workflow |
| fuzzing | `cargo +nightly fuzz run decode` in `proto/klatch` |

If you are about to add a CI job, ask: does it need to run on every pull request, and does it stay under a couple of
minutes? If not, make it manual and say how to run it here.

## Property tests

klatch uses [Hegel](https://hegel.dev) (the `hegeltest` crate, a dev-dependency, pinned exactly because it is beta
software) for property tests: instead of one example, a test states something that must hold for every input and Hegel
generates cases to try to break it. They live in `proto/klatch/tests/properties.rs` (pure functions) and
`proto/klatch/tests/models.rs` (model-based: a random sequence of claims, releases, posts, parks, reads and acknowledgements
is run against the real gateway and a tiny separate model, which must agree after every step; also the exhaustive role table). They run inside the
normal `cargo test`: the properties and the models take a few seconds together (100 cases each; about 10 s in the gate at 500). **A pass is evidence, not proof**: it means no
counterexample was found in the cases tried. Do not call anything property tested "proven".

**Run**

```
cargo test -p klatch --test properties                                  # the default 100 cases each
HEGEL_TEST_CASES=100000 cargo test -p klatch --test properties          # a long run, by hand (not in CI)
HEGEL_TEST_CASES=3000 cargo test -p klatch --test models                # 3000 sequences each: about half a minute
HEGEL_SEED=7 cargo test -p klatch --test properties                     # a fixed seed, to repeat a run exactly
```

**Write** (copy one from the file)

```rust
#[hegel::test]
fn name_says_what_always_holds(tc: TestCase) {
    let indices = tc.draw(gs::vecs(gs::integers::<u8>().max_value(6)));   // draw plain integers ...
    let value = value_from(&indices);                                      // ... and map them to values here
    assert!(property_of(value));
}
```

Draw plain integers, vectors and booleans and build the values you want from them inside the test: Hegel can print and
shrink those with no further setup (a `.map` closure or a helper returning `impl Generator` is not printable and the
compiler will tell you so). Generate *structured* inputs on purpose (path segments such as `..` and the empty string,
not arbitrary text, which almost never produces them). Use `tc.assume(condition)` to discard cases that do not apply.
Make the property strong enough to notice a bug: a property that a wrong implementation also satisfies proves nothing.
A good check is to plant the bug in the real code temporarily and watch the property fail.

**The planted-bug rule.** A property is kept only if you have shown it can fail: for each new property or model rule,
break the real code on purpose (one changed operator or dropped condition in the code it covers), run the test with a
fixed seed (`HEGEL_SEED=12345 HEGEL_TEST_CASES=500`), see it fail, then revert, and record the plant and the result in
RESEARCH-NOTES (section 42.8 has the table). A plant that is not caught means the property is too weak or the model
cannot reach the state: strengthen it (a model with no park or lease could never see a lease being shortened). Bound
every loop in a property (a bug that stops a pager advancing must fail the test, not hang it). Keep the whole property
run in the gate under about 15 seconds; slower properties belong in `--long` only.

**Replay a failure.** A failing property prints the smallest input it found and a line like
`#[hegel::reproduce_failure("AAUAAAAB...")]`. Add that attribute under `#[hegel::test]` and the failure replays from the
blob alone, with no luck involved, so a failure seen in CI can be replayed on your machine. Hegel also keeps failures in
`.hegel/examples/` (ignored by git) and replays them first on the next run in the same directory; delete the directory
to forget them. Remove the attribute once the bug is fixed.

**Why `static-engine`.** The dev-dependency turns on the `static-engine` feature. Without it the crate runs a nested
`cargo build` of its engine at build time, which resolves dependencies outside our lockfile where `--locked` and
`cargo-deny` cannot see them. Keep the feature on. (RESEARCH-NOTES section 42 has the measurements.)

## The Quint model of claims and leases (checked by hand; only the replay test runs in the gate)

`proto/klatch/models/claims.qnt` is a [Quint](https://quint-lang.org) model of the claim and lease protocol of one
room: overlap and exclusive or shared scopes, expiry, renewal to a claim's own ttl, liveness leases, parking, the 24
hour cap and the cycle-of-waits (deadlock) rule (DESIGN section 32 item 76). Its invariants have names
(`NoOverlappingLiveExclusive`, `LeaseWithinCap`, `RenewNeverRevives`, `DeadlockFlagExact` and others, listed at the
foot of the file) and are checked by [Apalache](https://apalache-mc.org). **Say "model checked up to N steps and M
members", never "proven"**: Apalache covers every behaviour up to a bound, and the model leaves out asks, anchors, rooms
and membership changes (the header of the file lists what).

```
proto/scripts/quint-check                 # random runs, then Apalache on each invariant (8 steps; about half an hour)
proto/scripts/quint-check findings        # the known findings: each is EXPECTED to be violated, and has a ticket
proto/scripts/quint-check mutations       # plant each bug in the model: each must be caught by its invariant
proto/scripts/quint-check mutate capOff   # one planted bug
proto/scripts/quint-check traces          # rewrite the traces in proto/klatch/models/traces
proto/scripts/quint-check versions
```

Install the tools outside the repository: `npm i --prefix ~/.local/share/klatch-quint @informalsystems/quint` (last
run with Quint 0.33.1; set `QUINT_BIN` to use another path), and Quint downloads Apalache into `~/.quint` the first
time it verifies (last run with Apalache 0.62.4, Node 22, Java 22). When you change claim, renewal, park or lease
behaviour, change the model with it, run `quint-check`, and run `quint-check traces` if the model's actions changed.

The link to the real code is `proto/klatch/tests/model_replay.rs`. It reads the traces (ITF JSON, one witness per
event from Apalache and some long random ones from `quint run`) and executes every step against `Gateway` over
`SqliteStore::open_in_memory`: a claim, release, read (renewal), post, `parked:` post, or a liveness probe made up by
the test. After each step the outcome (taken, refused by whom, told to yield, over the cap) and the live claims with owner,
scope and expiry must equal the model's. The few committed traces replay in an ordinary `cargo test` in well under a
second; `KLATCH_MODEL_TRACES=dir cargo test -p klatch --test model_replay` replays traces you just made.

## Before you open a pull request: the local gate

```
proto/scripts/gate            # about a minute; stops at the first failure and says how to replay it
proto/scripts/gate --long     # 5000 property cases and sequences each, by hand, takes minutes
```

It runs, in order: `cargo fmt --check`, clippy with warnings as errors, the workspace tests, `cargo deny` (skipped, with
a note, if it is not installed), the test-clock tests, the property tests and models with a bigger budget (500 cases
each, one fixed seed), and the repository's own checks (release numbers agree, the installer, the README quickstart,
shellcheck). The Hegel seed comes from the commit you are on, so a failure repeats exactly and each commit tries
different cases; when a Hegel step fails the gate prints the exact `HEGEL_SEED=... cargo test` command to replay it.

It is stricter than CI on purpose, because CI stays light (see above): CI runs the cheap subset on every pull request
and does not call this script. To run the gate automatically before every push, opt in with
`proto/scripts/install-git-hook` (it writes a pre-push hook, never touches a hook it did not write, and
`--uninstall` removes it; `git push --no-verify` skips it). Nothing installs the hook for you.
`proto/scripts/test-scripts.sh` tests the gate and the installer themselves.

### Running a team of headless workers

`proto/scripts/team` runs each task of a tasks file as its own headless Claude Code session (`claude -p`), a distinct verified klatch member with its own credential, git worktree and branch (`alee/<name>`) and hooks. Run it yourself, in your own terminal: issuing credentials refuses inside a coding-agent session by design.

```
cat > tasks.txt <<'EOF'
# name | ticket | prompt
alpha | 101 | Build the thing described in the ticket; open a PR.
EOF
! proto/scripts/team start tasks.txt     # from a Claude Code prompt; or without the ! in a plain terminal
proto/scripts/team status                # running, done, stopped or failed; exit code; cost
proto/scripts/team logs alpha            # tail of the stream-json log
proto/scripts/team stop                  # kill all workers and revoke their credentials
proto/scripts/team smoke                 # one tiny Haiku worker in a throwaway home first
```

Settings are environment variables listed at the top of the script (budget `TEAM_MAX_BUDGET`, wall clock `TEAM_MAX_SECONDS`, `TEAM_MODEL`, `TEAM_ROOM`, `TEAM_LAUNCHER`). `proto/scripts/test-team.sh` tests it with a fake klatch and a fake claude.

## Working on klatch

- Run `proto/scripts/gate` before you open a pull request. Branch and open a pull request; never push to `main`. Merge only
  after every check is confirmed green.
- Write the test first and watch it fail; say so in the pull request if a test was not red first.
- `cd proto && cargo fmt && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` before you
  push (CI fails on a formatting difference).
- Property tests (Hegel) are in `proto/klatch/tests/properties.rs`; see "Property tests" above.
- A bug, gap or limitation you find gets a GitHub issue, even if you fix it in the same pull request.
- Parallel pull requests must not share a line (#502). **CHANGELOG:** add a fragment file in `changelog.d/` (see its
  README) and leave the Unreleased section of CHANGELOG.md alone. **DESIGN.md numbered items:** fetch `origin/main`,
  take the next free number, and reserve it before you write it, by a klatch claim on `design-item:<N>` in the room
  where you coordinate (a second agent whose claim is refused takes the next number); if two pull requests still
  take the same number, the one merged second renumbers its own item. Never renumber an item already on main.
- Releases: see [RELEASING.md](RELEASING.md); versions: [VERSIONING.md](VERSIONING.md).
