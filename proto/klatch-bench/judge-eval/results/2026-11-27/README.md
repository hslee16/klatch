# Rules judge after the #1413 screen, 2026-11-27

The default rules judge gained a screen (`proto/klatch/src/domain/screen/`) on top of the credential scanner and the
scrub list: personal contact details and labelled identifiers, text that addresses the judge or reviewer, internal host
names and private addresses, confidentiality phrases, and a scrub match through lookalike letters, invisible characters
and line breaks (a term found only backwards is a hold). The bench `RulesJudge` calls the same function, so this is the
code `klatch judge run` runs. No model, no API, no credential, no network, no cost. Contested items are excluded as in
every judge-eval figure.

## What a hit means, and how the bench counts it

A credential or a scrub term found as written is a **deny** (final). Every other screen hit is a **deferral with a note**
naming the rule: the item stays queued for the reviewer or the model judge. The bench rules judge returns `Deny` for the
first, `Defer` for the second and `Allow` for text it did not match (which `klatch judge run` also defers, without a
note). Under both readings (#1546) a screen hold is a Defer, not a leak: `not_stopped_rate` counts only the items the
rules did not match, so **the miss rate means "not flagged" (denied or held), the same quantity as before the screen
changed from deny to hold, and its values did not change because of that switch**. What did change: the usual
"over-blocking" figure no longer counts a held clean item (a hold is not an over-block), so the report has a new line,
`flagged by the rules`, with clean items denied or held (the false-positive figure below) and should-not-cross items
flagged. "Blocked by rules" in the semantics block now counts only denies; held items are in "deferred".

## Two scorings of the test split (read this)

- **First version** (`first-rules-*.summary.json`, commit `39438e6b`; every hit was a deny): rules written from dev, test
  scored once, test miss and false-positive lists read to report them.
- **Current version** (`rules-*.summary.json`): after an independent review found the first version stopped 30 of 50
  harmless engineering messages and made every hit final, rules were narrowed and hits became deferrals. The changes were
  driven by the reviewer's false-positive list, not by test items, and no rule was tuned on a test item. But the test split
  has been scored twice, the second time after changes made with the first scoring's lists in view, so it is no longer
  an untouched holdout. Recall went down; the lower figures are reported.

## Items that should not cross, not flagged by the rules

| corpus | split | before the screen | first version | current |
|---|---|---|---|---|
| main | dev | 41 of 79, 51.9 percent | 5 of 79, 6.3 percent | 7 of 79, 8.9 percent [4.4, 17.2] |
| main | test | 30 of 55, 54.5 percent | 14 of 55, 25.5 percent | 18 of 55, 32.7 percent [21.8, 45.9] |
| hard | dev | 72 of 94, 76.6 percent | 10 of 94, 10.6 percent | 17 of 94, 18.1 percent [11.6, 27.1] |
| hard | test | 56 of 72, 77.8 percent | 26 of 72, 36.1 percent | 27 of 72, 37.5 percent [27.2, 49.0] |

## Clean items flagged (false positives; 0 in all four before the screen)

| corpus | split | first version | current |
|---|---|---|---|
| main | dev | 1 of 118 | 1 of 118, 0.8 percent [0.1, 4.6] |
| main | test | 1 of 79 | 0 of 79, 0 percent [0.0, 4.6] |
| hard | dev | 4 of 66 | 4 of 66, 6.1 percent [2.4, 14.6] |
| hard | test | 4 of 38 | 4 of 38, 10.5 percent [4.2, 24.1] |

The corpus false positives are messages that quote or discuss an injection phrase (the rules cannot tell a quote from an
attack and flag both: #1732), a seed-data file with an example email and phone number (hpii-017), and a partly masked IBAN
ending (hpii-009). A separate set of 70 synthetic, obviously fake engineering messages (the unit test in
`proto/klatch/src/domain/screen/tests.rs`, data in `benign.rs`) is flagged at none of three crossings; it was written after
the review found the false positives, so it pins them and is not a measurement.

Row-level results are not committed; the summaries (with the `semantics` block, now including the flagged counts) are here.

## What is still not flagged (counts of items that should not cross, dev and test, current version)

- main: business-private 5 and 4; personal-data 1 and 3; injection 0 and 8; multilingual 0 and 3; long 1 and 0.
- hard: personal-data 4 and 4; business-private 4 and 5; injection 2 and 9; multilingual 3 and 3; long 2 and 4; short 1 and
  1; secret-credential 1 and 0 (a value after the word passphrase); one scrub variant (spaced letters) on test.
