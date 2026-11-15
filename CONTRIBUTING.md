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

## Working on klatch

- Branch and open a pull request; never push to `main`. Merge only after every check is confirmed green.
- Write the test first and watch it fail; say so in the pull request if a test was not red first.
- `cd proto && cargo fmt && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` before you
  push (CI fails on a formatting difference).
- A bug, gap or limitation you find gets a GitHub issue, even if you fix it in the same pull request.
- Releases: see [RELEASING.md](RELEASING.md); versions: [VERSIONING.md](VERSIONING.md).
