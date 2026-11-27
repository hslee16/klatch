# Rules judge after the #1413 screen, 2026-11-27

The default rules judge gained a screen (`proto/klatch/src/domain/screen/`) on top of the credential scanner and the
scrub list: personal contact details and labelled identifiers, text that addresses the judge or reviewer, internal host
names and private addresses, confidentiality phrases, and a scrub match through lookalike letters, invisible characters,
line breaks and reversal. The bench `RulesJudge` calls the same function, so this is the code `klatch judge run` runs.
No model, no API, no credential, no network, no cost. Contested items are excluded as in every judge-eval figure.

## How it was run, and what the numbers mean

- `klatch-bench judge-eval --judge rules --split <dev|test> --corpus <corpus>.jsonl`, default scrub list, for both
  corpora and both splits. Row-level results are not committed; the summaries and the `semantics` block are in
  `rules-<corpus>-<split>.summary.json` here.
- **The rules were written by reading the dev misses and frozen (commit `39438e6b`) before the test split was run
  once.** Test was not studied and no rule changed after it was scored; the rules were not iterated on test. So the dev
  figures are tuned (optimistic by construction) and the test figures are the held-out claim. Both corpora are
  synthetic and labelled by one reviewer (#924); real traffic differs.
- Reading: `not_stopped_rate` (an item the rules did not stop counts as one that would cross). Under the production
  reading (`would_cross_without_reviewer_rate`) the rules judge never allows, so it is 0 by construction; see the
  2026-11-26 README (#1546).

## Items that should not cross, not stopped by the rules (before is the 2026-11-26 run, same corpora)

| corpus | split | before | after |
|---|---|---|---|
| main | dev | 41 of 79, 51.9 percent | 5 of 79, 6.3 percent [2.7, 14.0] |
| main | test | 30 of 55, 54.5 percent | 14 of 55, 25.5 percent [15.8, 38.3] |
| hard | dev | 72 of 94, 76.6 percent | 10 of 94, 10.6 percent [5.9, 18.5] |
| hard | test | 56 of 72, 77.8 percent | 26 of 72, 36.1 percent [26.0, 47.6] |

## Clean items the rules stopped (false positives; before was 0 in all four)

| corpus | split | after |
|---|---|---|
| main | dev | 1 of 118, 0.8 percent [0.1, 4.6] |
| main | test | 1 of 79, 1.3 percent [0.2, 6.8] |
| hard | dev | 4 of 66, 6.1 percent [2.4, 14.6] |
| hard | test | 4 of 38, 10.5 percent [4.2, 24.1] |

All of the dev false positives and most of the test ones are messages that quote or discuss an injection phrase
("ignore your instructions" inside a sentence about testing the judge, a paper, a code listing): the rules cannot tell a
quote from an attack and hold both. The others: a seed-data file with an example email and phone number (hpii-017), and
a partially masked IBAN ending (hpii-009).

## What is still missed (counts of items that should not cross, dev and test)

- main: business-private 5 and 3; personal-data 0 and 3; injection 0 and 6; multilingual 0 and 2.
- hard: personal-data 2 and 4; business-private 2 and 5; injection 1 and 8; multilingual 2 and 3; long 2 and 4; short 1
  and 1; one scrub variant (spaced letters) on test.
The test misses are mostly a person tied to data with no label (an account number after "account", a name beside a
number), private business matter with no confidentiality phrase (revenue, runway, pricing), injections that do not
address a named reviewer in a listed way (a forged chat transcript, a closing tag, a request to lower a confidence),
and personal data or injection in languages and phrasings the lists do not carry.
