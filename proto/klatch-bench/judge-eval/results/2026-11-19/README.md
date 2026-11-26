> Newer rules-judge figures, under both the old and the production (deferred is held) semantics, and a note that this run predates #921 and #987: see [../2026-11-26/README.md](../2026-11-26/README.md).

# Judge evaluation results, 2026-11-19, corpus 1 (333 items)

First full scoring of four judges on `corpus.jsonl` after the label-policy changes of #932 (contested items, name
spans, the `look-014` fix). Results recorded before #932 are not comparable. Row-level results are not committed
(they are large and carry no text); the per-run summaries are in this directory as `<judge>-<split>.summary.json`.

## How it was run

- Code: `klatch` main at `316eb27` (after #932), release build of `klatch-bench` with `--features jev-http`.
- Corpus: `judge-eval/corpus.jsonl`, frozen test digest `bf780a16c19969cc`. Dev 198 items, test 135. Contested items
  (`short-007`, `short-008`) are judged but excluded from every figure below.
- Claude: `claude` CLI 2.1.327, `claude:sonnet` and `claude:opus` (the aliases the CLI resolved at run time), one
  headless call per item, isolated settings.
- Jev: `jev-1.14.0`, confidence threshold 0.01 so every pick counts and the threshold table sees all of them.
- Commands (from `proto/`): `klatch-bench judge-eval --judge <rules|claude:sonnet|claude:opus> --split <dev|test>`
  and `doppler run --project governingdocs --config dev_alex -- klatch-bench judge-eval --judge jev:jev-1.14.0:0.01
  --split <dev|test>`. Claude was not run on the test split before the dev labels were checked, and the test split was
  studied only for items where both Claude models disagreed with the label.

## Results

"Strict leak" is a redact or deny item that crossed (allowed, or redacted with a span left); the denominator is the
items that should not cross (79 dev, 55 test). Over-block is a clean item redacted or denied, out of 118 dev and 79
test. Data leak ignores a leftover person's name once the data is gone.

| judge | split | n | accuracy | strict leaks | data leaks | over-blocks | p50 latency | cost |
|---|---|---|---|---|---|---|---|---|
| rules | dev | 197 | 74.1% | 51/79 (64.6%) | 51/79 | 0/118 | 0 ms | free |
| jev | dev | 197 | 88.3% | 17/79 (21.5%) | 17/79 | 1/118 | 148 ms | no cost reported |
| sonnet | dev | 197 | 99.5% | 1/79 (1.3%) | 0/79 | 0/118 | 3.1 s | $0.67 |
| opus | dev | 197 | 96.4% | 7/79 (8.9%) | 0/79 | 0/118 | 3.1 s | $1.20 |
| rules | test | 134 | 73.1% | 36/55 (65.5%) | 36/55 | 0/79 | 0 ms | free |
| jev | test | 134 | 80.6% | 16/55 (29.1%) | 16/55 | 0/79 | 154 ms | no cost reported |
| sonnet | test | 134 | 100.0% | 0/55 | 0/55 | 0/79 | 3.2 s | $0.57 |
| opus | test | 134 | 97.0% | 4/55 (7.3%) | 0/55 | 0/79 | 3.0 s | $1.31 |

Wilson 95% intervals are in the summaries and the runner's report; for scale, a 0 of 55 leak rate is 0% to 6.5%.

## What the results say

- The evaluation separates "no model" from a model judge: rules leak about two thirds of what should not cross.
- Sonnet and Opus cannot be separated. All of Opus's strict leaks are redactions that removed the data and kept a name
  (data leaks are 0 of 79 and 0 of 55). The corpus is too easy for strong judges (4 to 7 wrong items); a harder corpus
  is being built (`corpus-hard.jsonl`).
- Jev alone is not a judge: 21 to 29 percent of items that should not cross leak, with weak redaction and weak
  injection handling (test: 1 of 5 overt injections right). It is fast (about 150 ms against 3 s for Claude).
- Jev's confidence is informative. Its threshold table (a pick below the threshold becomes a defer) reaches a leak
  rate of at most 1 percent at 0.95 on dev (166 of 197 deferred) and at 0.80 on test (72 of 134 deferred). The two
  splits disagree, so the threshold is not yet trustworthy; this is why a cascade (Jev first, a stronger judge for
  what it defers) is not tuned until the hard corpus has run.
- The `scrub-term` category scores badly for every model judge because the gateway enforces the scrub list before any
  judge sees a message; it says nothing about a judge. It is still in the figures above and should be reported apart.

## Limits

Synthetic corpus, one author's labels (an independent labeller is #924), small categories (a category of ten moves
ten points per item), no hard cases yet, Jev and Claude run once each (no run-to-run variance measured).
