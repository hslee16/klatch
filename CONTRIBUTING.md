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
| release dry run (every build a release does, nothing published) | Actions, Release dry run, Run workflow |
| fuzzing | `cargo +nightly fuzz run decode` in `proto/klatch` |

If you are about to add a CI job, ask: does it need to run on every pull request, and does it stay under a couple of
minutes? If not, make it manual and say how to run it here.

## Property tests

klatch uses [Hegel](https://hegel.dev) (the `hegeltest` crate, a dev-dependency, pinned exactly because it is beta
software) for property tests: instead of one example, a test states something that must hold for every input and Hegel
generates cases to try to break it. They live in `proto/klatch/tests/properties.rs` (pure functions) and
`proto/klatch/tests/models.rs` (model-based: a random sequence of claims, releases, posts, reads and acknowledgements
is run against the real gateway and a tiny separate model, which must agree after every step). They run inside the
normal `cargo test`: the properties take well under a second, the models about a second (100 sequences each). **A pass is evidence, not proof**: it means no
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

**Replay a failure.** A failing property prints the smallest input it found and a line like
`#[hegel::reproduce_failure("AAUAAAAB...")]`. Add that attribute under `#[hegel::test]` and the failure replays from the
blob alone, with no luck involved, so a failure seen in CI can be replayed on your machine. Hegel also keeps failures in
`.hegel/examples/` (ignored by git) and replays them first on the next run in the same directory; delete the directory
to forget them. Remove the attribute once the bug is fixed.

**Why `static-engine`.** The dev-dependency turns on the `static-engine` feature. Without it the crate runs a nested
`cargo build` of its engine at build time, which resolves dependencies outside our lockfile where `--locked` and
`cargo-deny` cannot see them. Keep the feature on. (RESEARCH-NOTES section 42 has the measurements.)

## Working on klatch

- Branch and open a pull request; never push to `main`. Merge only after every check is confirmed green.
- Write the test first and watch it fail; say so in the pull request if a test was not red first.
- `cd proto && cargo fmt && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` before you
  push (CI fails on a formatting difference).
- Property tests (Hegel) are in `proto/klatch/tests/properties.rs`; see "Property tests" above.
- A bug, gap or limitation you find gets a GitHub issue, even if you fix it in the same pull request.
- Releases: see [RELEASING.md](RELEASING.md); versions: [VERSIONING.md](VERSIONING.md).
