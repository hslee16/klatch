# PR sweep

`proto/scripts/pr-sweep` runs ONE pass over `pr-ledger` and takes the next step on every PR that has one, so a green PR does not sit unmerged until someone remembers (#702, #706). Run it in a loop to keep it standing.

## Run it

```
proto/scripts/pr-sweep --dry-run     # print what it would do; changes nothing
proto/scripts/pr-sweep               # one real pass
proto/scripts/pr-sweep --bench-ledger FILE   # keep the benched-PR list somewhere else
proto/scripts/pr-sweep --budget-floor 1500   # skip the pass (exit 0, with a message) when fewer than 1500 API calls are left
proto/scripts/pr-sweep --batch [--max N]     # land several PRs per integration (see Batch mode below)
```

Use a clone or worktree that nobody else is editing in: integrating switches that checkout to a throwaway branch `alee/integ-N` (it refuses a dirty tree). Needs `gh` logged in, `python3`, and `cargo` (it adds `$HOME/.cargo/bin` to PATH itself; it never prints the environment).

The bench ledger defaults to `$TMPDIR/pr-sweep-bench.json`. A fresh ledger makes every recently merged PR show as `merged-not-benched`, so the first real pass would bench them all; seed it first with `python3 -I proto/scripts/pr-ledger --record-bench N --bench-ledger FILE` for the PRs already benched.

## What it does, per stage

| Stage | Action |
| --- | --- |
| checks-cancelled | `pr-ledger --rerun-cancelled PR` |
| ready | re-checks live that every check is green on the latest head and the PR is mergeable; if behind, integrates main first and waits for green on the new head; merges with `gh pr merge PR --merge --match-head-commit SHA`; runs `cargo run -q --release -p klatch-bench` (must say "N passed, 0 failed") and records it with `--record-bench`; closes the linked ticket if still open |
| behind, conflict | merges origin/main into the PR branch on `alee/integ-N` (conflicts in DESIGN.md, CHANGELOG.md, VERSIONING.md, scenarios `mod.rs` keep both sides, DESIGN items sorted by number; new design items are fragments in `design/items/` (#807), so a DESIGN.md conflict is now rare, but PRs that still append a numbered item keep resolving this way), runs the gate, pushes to the PR's own branch |
| merged-not-benched, merged-open-ticket | bench and record; close the ticket |
| checks-red, code conflict, no ticket, stuck over 3 h, abandoned, failed gate or bench | not touched; one lobby message via `pr-ledger --post --only` names them (only when the list changed) |

It exits 0 with one summary line (`nothing needs a step` when there is nothing to do).

## Batch mode (#852)

```
proto/scripts/pr-sweep --batch --dry-run   # print the plan: which PRs, which dropped and why; changes nothing
proto/scripts/pr-sweep --batch [--max N]   # default N is 6, minimum 2
```

One at a time, every merge makes every other ready PR fall behind, so a queue of 10 takes hours. `--batch` lands several PRs per integration. The human's rule (2026-11-18): no merge conflicts before merges.

1. **Candidates**: PRs at stage `ready` or `conflict` with a linked ticket and every check green on their own head, in PR-number order, at most `--max`. A conflict only counts when it is in a docs file the resolver handles (DESIGN.md, CHANGELOG.md, VERSIONING.md, RESEARCH-NOTES.md, scenarios `mod.rs`).
2. **Pre-check**: the sweep builds a throwaway branch `alee/batch-TIMESTAMP` from `origin/main`. Before each candidate touches it, `git merge-tree --write-tree --name-only` (needs git 2.38 or newer; it only writes objects) says which files would conflict, first against `origin/main`, then against the branch so far. A conflict in any non-docs file drops the PR BEFORE it is merged: it is reported, posted via `pr-ledger --post --only`, and never merged. Two PRs that conflict with each other: the lower number stays, the higher is dropped. A head that moved since it was seen green is dropped too. The survivors are added with a real `git merge` of `refs/pr-sweep/N` (fetched with `--no-prune`); docs conflicts keep both sides; a merge that still conflicts is aborted and the PR dropped.
3. **Fewer than 2 survivors**: nothing is batched; the one-at-a-time path takes over for the survivors.
4. **Gate once** on the combined branch. If it fails the sweep says `batch gate failed`, pushes nothing, and the next pass takes that same set one at a time (the failed set is remembered in `$TMPDIR/pr-sweep-batch-failed`; a different set batches again). Bisecting is out of scope.
5. **One pull request** `Batch: #A #B #C` is opened from the pushed batch branch. Its body lists the PRs and copies each one's `Closes`/`Fixes`/`Resolves`/`Refs` lines, so tickets still close, and ends with the Generated-with line. The sweep waits for BOTH checks green on that head, then merges with `gh pr merge --merge --match-head-commit SHA`. If main moved (the existing is_behind rule) it merges main into the batch branch, re-gates, pushes and waits again, always merging the new head; a code conflict with main closes the batch PR with a comment and posts every included PR.
6. **After the merge**: each included PR's head is checked with `git merge-base --is-ancestor HEAD origin/main` and a REST read. GitHub normally marks them merged on its own; a PR still open after a short wait whose head is in main gets the comment `merged via #BATCH` and is closed with REST (`PATCH state=closed`). A still-open PR whose head is NOT in main is never closed, it is posted. Then the bench runs once (must say `N passed, 0 failed`), every included PR is recorded as benched, `merged: #N` is posted for each, the tickets the PRs close (Closes/Fixes/Resolves only) are closed, and one lobby summary is posted.

Safety rules stay: only PRs green on their own head, no force-push, no push to main (only `alee/batch-*` and a PR's own branch are pushed), no `git merge` on main, one sweep at a time via the lock, a real failure is never rerun.

If a pass dies with its batch PR still open (checks not green in time), the next pass sees the open `alee/batch-*` PR, starts no new batch, leaves the PRs listed in its body alone, and lands the batch PR the ordinary way (the included PRs are then benched one by one as `merged-not-benched`).

`--dry-run` fetches refs only (`refs/pr-sweep/*`, `origin/main`); it never switches branches, merges, commits, pushes, opens a PR or posts. It checks each candidate against main and against the earlier survivors in pairs.

## When a call fails (#754, #756)

- Every `gh` call has a 60 s timeout and two retries (a hung call once stalled a pass for 33 minutes). A rate limit (HTTP 403, "rate limit") is not retried.
- A call that still fails, or a ledger that failed, makes the pass incomplete: the sweep prints `N call(s) failed, pass incomplete` (and `GitHub API rate limit exhausted; resets at TIME` for a rate limit) and exits 1. It never prints `nothing needs a step` after a failure, and a PR whose checks could not be read is never treated as "not green".
- `--budget-floor N` reads `gh api rate_limit` (it costs no call) and skips the pass with exit 0 when fewer than N calls are left. Use 1500 in the loop.
- A lock held by a live pid for over 30 minutes is reported as `held the lock for N minutes ... it may be hung: kill -9 PID` and the sweep exits 3, so the loop line shows it. The next pass takes the lock once that pid is gone.
- `pr-ledger` makes few calls per PR: the open and recently merged lists once each, one check-runs call per open PR that is not a draft, the pull itself only for a PR whose checks are all green (for its mergeable state), and the ticket only for a merged PR. Agents poll it at most every 3 minutes with `--budget-floor 1000` (AGENT-PR-RULES.md).

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
/loop 10m proto/scripts/pr-sweep --budget-floor 1500
```

Each tick is one pass. Run `--dry-run` first and read the output before switching the loop on.

To batch, the coordinator changes the loop line to `/loop 10m proto/scripts/pr-sweep --batch --budget-floor 1500` once a `--batch --dry-run` plan looks right. Rows that are not candidates (cancelled reruns, merged-not-benched, red, no ticket) are handled as before in the same pass.

## Tests

Files marked `merge=union` in `.gitattributes` (#1065: `proto/AUTHZ-AUDIT.md`, the `mod x;` registries) are merged by git itself, in the sweep's real `git merge` and in the `git merge-tree` pre-check, so they never reach the docs resolver; the resolver and its docs list stay as the fallback for the files git cannot union (DESIGN.md, CHANGELOG.md, and two edits of one line of the others). GitHub ignores the attribute, so a PR can show `dirty` there and merge cleanly here.

`proto/scripts/test-sweep.sh` (run by the gate) drives the sweep with a fake `gh`, `git`, ledger, gate and bench on PATH; nothing touches the network or a real repo.
