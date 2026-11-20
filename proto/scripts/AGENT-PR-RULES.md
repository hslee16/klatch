# PR rules for agents

Paste this block into the prompt of any agent that is not started by `proto/scripts/team` (which already puts the same rules in its brief). Replace `N` and `PR` with the real numbers, and `K` with the klatch binary.

```
## After you open the PR (do not stop at "waiting on background work")
- The PR body says "Closes #N" for your ticket (when you have one), so pr-ledger links the ticket.
- Poll your own PR with `python3 -I proto/scripts/pr-ledger --budget-floor 1000` (it lists every open PR with a stage and a next step; find yours). Poll at most once every 3 minutes: GitHub allows 5000 API calls an hour for every agent together, and each poll costs several. Use `K wait lobby --timeout 180` between polls, never sleep. `--budget-floor 1000` reads `gh api rate_limit` (free) first and skips with a message when fewer than 1000 calls are left; then wait for the reset time it names. A line saying pass incomplete means a call failed, not that nothing needs a step. Keep going until your PR reaches `ready`.
- A stage of checks-cancelled means jobs were cancelled before they ran: rerun them with `python3 -I proto/scripts/pr-ledger --rerun-cancelled PR`.
- A stage of checks-red is a real failure: read the failing job (the ledger shows its first error line), fix it, push. Never rerun a real red check.
- At `ready`, post "ready: #PR" in the room and release your claims. You still never merge; the human or coordinator does.
- A new DESIGN item goes in its own file `design/items/<ticket>-<slug>.md` (line 1 is the item title, the rest is the text; see design/items/README.md); never append to DESIGN.md and never pick an item number (#807). Refer to an item that is not yet assembled as "the design item for #NNN".
- A new RESEARCH-NOTES section goes in its own file `research/notes/<ticket>-<slug>.md` (line 1 is the section title, `###` headings are subsections; see research/notes/README.md); never append a section to RESEARCH-NOTES.md and never pick a section number (#859). Refer to a note that is not yet assembled as "the research note for #NNN".
- When you open the PR, in the same step post `pr: #PR <title>` in the room and run `K claim lobby pr:PR --shared`; renew it every 10 minutes until the PR is over (DESIGN item 84). A PR you never post is invisible on the board.
- When it is merged (or closed without merging), post `merged: #PR` and release `pr:PR --note merged`. If you must stop first, release `pr:PR` without that note and say so in a `handoff:` post. When the lobby digest shows an open PR without an owner, claim it before starting new work.
```

pr-sweep posts `merged: #PR` to the lobby after it merges a PR. The coordinator can also run `python3 -I proto/scripts/pr-ledger --post` to post the PRs that need a step to the lobby (one message, only when the list changed), and `--record-bench PR --bench-ledger FILE` after benching a merged PR (then list with `--bench-ledger FILE`, so a merged PR not yet benched shows as `merged-not-benched`).
- Never claim a shared document (DESIGN.md, CHANGELOG.md, RESEARCH-NOTES.md). Add a fragment file (`design/items/`, `changelog.d/`) instead; claim only the code paths you rewrite (#533).
