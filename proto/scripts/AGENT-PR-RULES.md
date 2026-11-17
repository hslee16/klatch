# PR rules for agents

Paste this block into the prompt of any agent that is not started by `proto/scripts/team` (which already puts the same rules in its brief). Replace `N` and `PR` with the real numbers, and `K` with the klatch binary.

```
## After you open the PR (do not stop at "waiting on background work")
- The PR body says "Closes #N" for your ticket (when you have one), so pr-ledger links the ticket.
- Poll your own PR with `python3 -I proto/scripts/pr-ledger` (it lists every open PR with a stage and a next step; find yours). Use `K wait lobby --timeout 60` between polls, never sleep. Keep going until your PR reaches `ready`.
- A stage of checks-cancelled means jobs were cancelled before they ran: rerun them with `python3 -I proto/scripts/pr-ledger --rerun-cancelled PR`.
- A stage of checks-red is a real failure: read the failing job (the ledger shows its first error line), fix it, push. Never rerun a real red check.
- At `ready`, post "ready: #PR" in the room and release your claims. You still never merge; the human or coordinator does.
```

The coordinator can also run `python3 -I proto/scripts/pr-ledger --post` to post the PRs that need a step to the lobby (one message, only when the list changed), and `--record-bench PR --bench-ledger FILE` after benching a merged PR (then list with `--bench-ledger FILE`, so a merged PR not yet benched shows as `merged-not-benched`).
