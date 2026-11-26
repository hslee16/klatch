# Rules judge under two semantics, 2026-11-26 (#1546)

The bench `RulesJudge` answers Allow for text it does not match. `klatch judge run` does not: it defers that text
(DESIGN item 141), so it stays queued for the quarantined reviewer and does not cross without one. The figures
published so far (the 51.9 and 54.5 percent of SECURITY.md) therefore mean "not stopped by the rules". This file
reports the same run under both readings and says which is which.

- `not_stopped_rate` (semantics `not-stopped-by-rules`): an unmatched item counts as one that would cross. This is
  the old `leak_rate` and is unchanged.
- `would_cross_without_reviewer_rate` (semantics `production`): an unmatched item is deferred, so it is held. Only
  an item the rules themselves allowed could cross, and the rules judge never allows, so this is 0 by construction.
  That says the rules do not vouch for anything; it does not say held items are safe, because a held item crosses if
  a reviewer later allows it, and the reviewer is not measured here.

## How it was run

- Code: klatch main at `7ae41503` plus the #1546 change, `klatch-bench judge-eval --judge rules --split <dev|test>`
  with `--corpus corpus.jsonl` and `--corpus corpus-hard.jsonl`, default scrub list. No model, no API, no
  credential, no network, no cost. Contested items are excluded as in every judge-eval figure.
- Row-level results are not committed; the per-run summaries and the `semantics` block are in
  `rules-<corpus>-<split>.summary.json` here.

## Results (items labelled as ones that should not cross)

| corpus | split | blocked by rules | deferred to reviewer | allowed by rules | not stopped (old reading) | would cross without reviewer (production) |
|---|---|---|---|---|---|---|
| main | dev | 38 | 41 | 0 | 41 of 79, 51.9 percent | 0 of 79 |
| main | test | 25 | 30 | 0 | 30 of 55, 54.5 percent | 0 of 55 |
| hard | dev | 22 | 72 | 0 | 72 of 94, 76.6 percent | 0 of 94 |
| hard | test | 16 | 56 | 0 | 56 of 72, 77.8 percent | 0 of 72 |

## Share that reaches a reviewer (all items, production reading)

Every item the rules do not block is deferred, clean ones included, so the reviewer sees all of them.

| corpus | split | items | blocked | deferred (reaches a reviewer) |
|---|---|---|---|---|
| main | dev | 197 | 38 | 159, 80.7 percent |
| main | test | 134 | 25 | 109, 81.3 percent |
| hard | dev | 160 | 22 | 138, 86.2 percent |
| hard | test | 110 | 16 | 94, 85.5 percent |

Of the items that should not cross, the share that reaches a reviewer is 41 of 79, 30 of 55, 72 of 94 and 56 of 72 (the
same counts as the old figure). Without a reviewer none of them crosses; with one, the reviewer is the control.

## Older results

The 2026-11-19 README and its `rules-*.summary.json` files (64.6 and 65.5 percent on the main corpus) predate the
credential scanner changes of #921 and #987 and use the old reading only. The 2026-11-23 S10 figures (51.9 and 54.5
percent) are the current code under the old reading and reproduce here exactly.
