# Working in this repository

Short list of the rules an agent follows here. Each links to the document that holds the detail.

## At the start of a session

If you coordinate (merge, sweep, report on open pull requests), run `proto/scripts/gh-watch ensure`, then arm a Monitor on
`tail -F <state dir>/gh-watch.log | grep --line-buffered -E 'ALERT|INFO'` and re-arm it when it expires (30 minutes). ALERT
lines are what you report. Keep the slow poll as the backstop. `ensure` starts only the receiver. Only the human starts the
forwarder (`gh webhook forward`), or an agent when the human explicitly asked for it in that session. Detail:
[proto/scripts/PR-SWEEP.md](proto/scripts/PR-SWEEP.md), "At the start of a session".

## Waiting

Never write an `until grep` or `sleep` loop to wait on an event or a file (14 duplicate loops were left behind by one builder
on 2026-11-22). For a long command start it once with `run_in_background`, post `parked:` and end the turn; one wait per long
command. To wait on a room use `klatch wait ROOM --only PREFIX --timeout S`. Background tasks and Monitors end with the
session. Detail: [INTEGRATION.md](INTEGRATION.md), "How an agent learns of events without a shell loop".

## Merging

Branch and open a pull request; never push to `main`. Merge only after the check on the PR head is green. Never force-push.
Never rebase: merge `main` into the branch instead. Sources: [CONTRIBUTING.md](CONTRIBUTING.md) ("Working on klatch"),
[proto/scripts/PR-SWEEP.md](proto/scripts/PR-SWEEP.md) ("What it never does"),
[proto/scripts/INTEGRATOR.md](proto/scripts/INTEGRATOR.md).

## Writing

No em dashes in anything you author (code comments, docs, fragments, commit messages). See
[changelog.d/README.md](changelog.d/README.md) and [design/items/README.md](design/items/README.md).

## Credentials

Agents never issue credentials, and never touch a live key store or `~/.klatch`; a person runs `klatch worker issue` and
`proto/scripts/team` (issuing refuses inside a coding-agent session by design). See [CONTRIBUTING.md](CONTRIBUTING.md)
("Running a team of headless workers") and [SECURITY.md](SECURITY.md).
