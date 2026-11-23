# Coordination bench

Does a team of agents coordinate better with klatch than without it? This directory holds the benchmark that answers it
(epic #1362). The rules are in [SPEC.md](SPEC.md): arms, scenarios S1 to S10, metrics, statistics, results schema,
conditions, thresholds (empty until the M0 baseline), milestones and threats to validity.

Status: specification only (#1363). No harness, no fixtures, no results yet. Nothing here runs an agent or spends money.

- `SPEC.md`: the specification.
- `arms/`: the agent prompts for each arm. DRAFT until the B1 ticket (#1365) has them reviewed by someone who did not
  write them; the klatch arms must not get a better prompt than the baselines.
- `fixtures/` (later, scenario tickets): small deterministic repositories.
- `checks/` (later): hidden checkers, never placed in a fixture.
- `results/` (later, #1364 and #1380): one JSON record per run and a ledger.

Related: `experiments/variance/repeat.py` (the repeat harness this builds on), `proto/klatch-bench` (the scripted bench),
`experiments/dm-adoption` (the adoption runs).
