# Judge evaluation results, 2026-11-19, hard corpus (271 items) and the cascade

Second results file, for `corpus-hard.jsonl` (#931) and the cascade tooling (#936). Corpus 1 results are in `README.md`
beside this file. Same method: contested items excluded, Wilson intervals in the per-run summaries
(`hard-<judge>-<split>.summary.json`), Claude CLI 2.1.327 with the `sonnet` and `opus` aliases, Jev `jev-1.14.0` at
threshold 0.01. Code at `klatch` main after #939; hard corpus test digest `f12ed130999e174f`.

These figures replace an earlier version of this file that scored the corpus before #939 (which relabelled five
name-plus-last-four-of-a-card items from allow to redact and marked `xinjl-005` contested). The earlier figures are not
comparable. Effect of the relabel: Sonnet's dev over-blocking fell from 6 of 71 to 1 of 66, because most of those
"over-blocks" were correct redactions.

## Results (strict leaks are over the items that should not cross: 94 dev, 71 test)

| judge | split | n | accuracy | strict leaks | data leaks | over-blocks | p50 latency | cost |
|---|---|---|---|---|---|---|---|---|
| rules | dev | 160 | 48.1% | 83/94 (88.3%) | 83/94 | 0/66 (0.0%) | 0.0 s | free |
| jev | dev | 160 | 70.0% | 26/94 (27.7%) | 26/94 | 12/66 (18.2%) | 0.1 s | not reported |
| sonnet | dev | 160 | 88.8% | 16/94 (17.0%) | 1/94 | 1/66 (1.5%) | 3.9 s | $1.45 |
| opus | dev | 160 | 82.5% | 28/94 (29.8%) | 0/94 | 0/66 (0.0%) | 3.8 s | $2.55 |
| rules | test | 110 | 42.7% | 63/71 (88.7%) | 63/71 | 0/39 (0.0%) | 0.0 s | free |
| jev | test | 110 | 58.2% | 24/71 (33.8%) | 24/71 | 7/39 (17.9%) | 0.1 s | not reported |
| sonnet | test | 110 | 87.3% | 10/71 (14.1%) | 1/71 | 2/39 (5.1%) | 3.9 s | $0.89 |
| opus | test | 110 | 86.4% | 13/71 (18.3%) | 0/71 | 2/39 (5.1%) | 3.9 s | $1.74 |

## What the hard corpus shows

- It separates the judge classes that corpus 1 could not: rules 43 to 48 percent, Jev 58 to 70 percent, Claude 82 to 89
  percent accuracy.
- Sonnet and Opus are close. Sonnet leads on dev (88.8 against 82.5) and on strict leaks; on test they are within three
  points (87.3 against 86.4, within the intervals). Opus costs about 1.7 to 2.9 times as much for no gain. Almost all of
  the strict-leak gap is redactions that removed the data and kept a person's name: data leaks are 0 or 1 of 94 and 0 or 1
  of 71 for both.
- Over-blocking of clean items is low for both Claude models after the relabel (0 to 5 percent).
- Jev alone is not a judge on hard items: it over-blocks 18 percent of clean items and leaks 28 to 34 percent.

## Cascade (Jev first, Sonnet second), simulated offline with `judge-eval cascade-sim`

Policy: a Jev deny is accepted at the deny threshold, a Jev allow only when probability and confidence both reach 0.95,
every redact and everything else goes to Sonnet. Thresholds are read on dev and scored once on test.

| corpus | split | policy | accuracy | strict leaks | over-blocks | escalated to Sonnet | mean latency | cost |
|---|---|---|---|---|---|---|---|---|
| corpus 1 | dev | Sonnet alone | 99.5% | 1.3% | 0% | 100% | 3.4 s | $0.67 |
| corpus 1 | dev | deny 0.5, allow 0.95 | 99.5% | 1.3% | 0% | 59.4% | 2.2 s | $0.42 |
| corpus 1 | test | Sonnet alone | 100% | 0% | 0% | 100% | 3.9 s | $0.57 |
| corpus 1 | test | deny 0.9, allow 0.95 | 100% | 0% | 0% | 82.1% | 3.5 s | $0.50 |
| hard | dev | Sonnet alone | 88.8% | 17.0% | 1.5% | 100% | 4.9 s | $1.45 |
| hard | dev | deny 0.9, allow 0.95 | 88.8% | 17.0% | 1.5% | 96.9% | 4.9 s | $1.43 |
| hard | dev | deny 0.7, allow 0.95 | 87.5% | 17.0% | 4.5% | 86.2% | 4.4 s | $1.25 |
| hard | dev | deny 0.5, allow 0.95 | 85.6% | 16.0% | 9.1% | 73.8% | 3.9 s | $1.09 |
| hard | test | Sonnet alone | 87.3% | 14.1% | 5.1% | 100% | 4.4 s | $0.89 |
| hard | test | deny 0.9, allow 0.95 | 86.4% | 15.5% | 5.1% | 96.4% | 4.4 s | $0.87 |

Reading: on easy, chat-heavy traffic the cascade keeps Sonnet's accuracy and saves 18 to 41 percent of Sonnet calls. On hard
traffic it saves 3 to 26 percent, and the low deny thresholds that save the most start to over-block (9.1 percent at 0.5
on dev). The absolute saving is small (Sonnet is about $0.003 to $0.009 per item), and the cascade sends every escalated
text to two hosted vendors. Recommendation recorded for the human: Sonnet alone as the hosted judge behind the
strictly-below-the-ceiling gate; keep the cascade as tooling.

## Label check

Corpus 1 and the hard corpus each had a pass where the model judges' disagreements with the stored labels were read
(corpus 1 in `README.md`). For the hard corpus the pass found one policy cluster (name plus last four of a card; relabelled
in #939) and one borderline injection look-alike (`xinjl-005`, now contested). `hpii-009` (an organisation, not a person,
with a partial IBAN) stays allow and is noted in `README-hard.md`.
