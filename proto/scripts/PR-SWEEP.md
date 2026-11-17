# PR sweep

`proto/scripts/pr-sweep` runs ONE pass over `pr-ledger` and takes the next step on every PR that has one, so a green PR does not sit unmerged until someone remembers (#702, #706). Run it in a loop to keep it standing.

## Run it

```
proto/scripts/pr-sweep --dry-run     # print what it would do; changes nothing
proto/scripts/pr-sweep               # one real pass
proto/scripts/pr-sweep --bench-ledger FILE   # keep the benched-PR list somewhere else
```

Use a clone or worktree that nobody else is editing in: integrating switches that checkout to a throwaway branch `alee/integ-N` (it refuses a dirty tree). Needs `gh` logged in, `python3`, and `cargo` (it adds `$HOME/.cargo/bin` to PATH itself; it never prints the environment).

The bench ledger defaults to `$TMPDIR/pr-sweep-bench.json`. A fresh ledger makes every recently merged PR show as `merged-not-benched`, so the first real pass would bench them all; seed it first with `python3 -I proto/scripts/pr-ledger --record-bench N --bench-ledger FILE` for the PRs already benched.

## What it does, per stage

| Stage | Action |
| --- | --- |
| checks-cancelled | `pr-ledger --rerun-cancelled PR` |
| ready | re-checks live that every check is green on the latest head and the PR is mergeable; if behind, integrates main first and waits for green on the new head; merges with `gh pr merge PR --merge --match-head-commit SHA`; runs `cargo run -q --release -p klatch-bench` (must say "N passed, 0 failed") and records it with `--record-bench`; closes the linked ticket if still open |
| behind, conflict | merges origin/main into the PR branch on `alee/integ-N` (conflicts in DESIGN.md, CHANGELOG.md, VERSIONING.md, scenarios `mod.rs` keep both sides, DESIGN items sorted by number), runs the gate, pushes to the PR's own branch |
| merged-not-benched, merged-open-ticket | bench and record; close the ticket |
| checks-red, code conflict, no ticket, stuck over 3 h, abandoned, failed gate or bench | not touched; one lobby message via `pr-ledger --post --only` names them (only when the list changed) |

It exits 0 with one summary line (`nothing needs a step` when there is nothing to do).

## What it never does

- Merge a PR whose checks are not all green on its latest push (the merge is pinned to the head it saw green), or one that is conflicted or has no ticket.
- Rerun a real failure (only `cancelled` jobs, which never ran).
- Force-push, push to main, or run `git merge` while on main.
- Overlap with itself: a lock directory `$TMPDIR/pr-sweep.lock` holds the pid of the running sweep; a second sweep exits at once. A lock whose pid is gone is taken over.
- Touch a PR the code cannot merge cleanly: a code conflict aborts the merge and the PR is posted.

The recorded human reading on #706 is that a green PR may be merged without a per-PR prompt under the standing rule (every check on the latest push green). Keep that gate strict.

## Run it as a loop

In a Claude Code session, in the clone it should use:

```
/loop 5m proto/scripts/pr-sweep
```

Each tick is one pass. Run `--dry-run` first and read the output before switching the loop on.

## Tests

`proto/scripts/test-sweep.sh` (run by the gate) drives the sweep with a fake `gh`, `git`, ledger, gate and bench on PATH; nothing touches the network or a real repo.
