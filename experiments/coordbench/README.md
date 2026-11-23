# Coordination bench

Does a team of agents coordinate better with klatch than without it? This directory holds the benchmark that answers it
(epic #1362). The rules are in [SPEC.md](SPEC.md): arms, scenarios S1 to S10, metrics, statistics, results schema,
conditions, thresholds (empty until the M0 baseline), milestones and threats to validity.

Status: the harness, the results ledger and the report generator exist (#1364) and run with fake agents. The real-agent
driver is built and its command and stream parser are unit tested, but no real run has been made: that needs the human's
written budget approval (SPEC section 8). Scenario fixtures other than the trivial test fixture are the scenario tickets'
work (#1366 to #1380), and the arm prompts are PROPOSED by #1365 and stay unused until the human has reviewed them (`arms/REVIEW.md`) and run the pilot (`arms/PILOT.md`).

- `SPEC.md`: the specification.
- `arms/`: the agent prompts for each arm (B0, B1, B1-roles, K1, K2, K3), `check_parity.py` (lengths, outline, shared prompt,
  no klatch words in B arms), `REVIEW.md` (the checklist for a reviewer who did not write them) and `PILOT.md` (the pilot
  plan). Proposed by #1365, unused until the human reviews them; the klatch arms must not get a better prompt than the baselines.
- `run.py`: runs one scenario against one or more arms, N times each (see Usage).
- `report.py`, `compare.py`: generate REPORT.md from a results directory; compare two results directories.
- `stats.py`: Fisher exact, Mann-Whitney U, Hodges-Lehmann, seeded bootstrap interval, Wilson, Newcombe and Holm, standard
  library only, tested against values from the literature.
- `streams.py`: parses the stream-json of an agent (turns, tokens, cost, tool calls, messages, conflicts).
- `drivers.py`, `fake_agent.py`: the agent driver interface with the `fake` (scripted, no model) and `claude` (headless
  `claude -p`) drivers.
- `fixture.py`, `fixtures/`, `checks/`: the fixture interface and the one test fixture `trivial`.
- `prices.json`, `cost_model.json`: the price table (cost estimates while a run is live) and the SPEC 8.1 planning figures
  (dry-run estimates). Both are planning data to verify before a real run.
- `results/`: the ledger and the run records (ignored by git, see its README).
- `tests/`: unit tests.

## Usage

Everything below uses only the Python 3 standard library. Nothing here starts a model unless you pass `--driver claude`.

Plan a suite and see the estimated cost range without running anything:

    python3 experiments/coordbench/run.py --fixture trivial --suite-id m0-try --arm B0,B1,K1,K2 --runs 10 --dry-run

Run the trivial scenario with fake agents across the arms (K arms need a built klatch; B arms do not):

    python3 experiments/coordbench/run.py --fixture trivial --suite-id try --arm B0,B1,K1,K2 --runs 3 \
        --klatch-bin "$PWD/proto/target/debug/klatch" --logs-dir /tmp/coordbench-logs

Generate the report, check the ledger, compare with a previous release:

    python3 experiments/coordbench/report.py experiments/coordbench/results/try
    python3 experiments/coordbench/report.py experiments/coordbench/results --check-ledger
    python3 experiments/coordbench/compare.py experiments/coordbench/results/release-0.3.0 \
        experiments/coordbench/results/release-0.2.0

A real run (not made yet) adds `--driver claude --model MODEL_ID --claude-bin PATH --budget-approved-in REF
--max-run-usd N --max-suite-usd N`. `--budget-approved-in` is the harness's guard for SPEC section 8: it refuses real agents
without it and writes the reference into the ledger header. The model API key (`ANTHROPIC_API_KEY`) is read from the
harness environment and passed to the agent processes of the claude driver only; it is never printed or written.

What a run does, in order: a private scratch directory under `--tmp-root` (mode 700) with an empty KLATCH_HOME and
KLATCH_KEY_DIR and its own HOME; the fixture's initial tree committed and pushed to a per-run bare `origin`; one fresh
clone per agent (K2 also runs `klatch init --scope project --yes` in each clone; B1 adds a `TASKS.md` board); the arm
prompt built from `arms/common.md` and `arms/<arm>.md`; the agents started as separate processes and left alone; the hidden
checker run on a clone of the final origin (and every `--poll-s` seconds, for `time_to_first_pass_s`); a record written,
whatever the outcome; the scratch removed. Arms are interleaved in seeded random blocks. A per-run spend cap, a wall cap and
a suite cap stop the work; the outcome is `spend-cap`, `timeout`, or a suite stop recorded in `suite.json` (exit code 3).
Exit codes: 0 all runs ok, 1 a run failed or was invalid, 2 refused, 3 suite cap hit.

Isolation: the harness refuses a store path under `~/.klatch` or `~/.claude`, outside the temp root, or reached through a
symlink. It runs the klatch CLI only by the `--klatch-bin` path and builds every child environment from an allowlist, so no
credential reaches klatch or a fake agent.

## Fixtures

A fixture is `fixtures/<name>/` with `fixture.json` (agents and their task files, test command, the checker name, optional
`est_cost_usd_haiku` and `fake` script), `setup.sh DEST SEED` (writes the initial tree; the harness does the git work) and the
task texts. The hidden checker is `checks/<checker>/check.sh CHECKOUT`: it lives outside the fixture so no checkout contains
it, prints `{"checks": [{"id": ..., "pass": true, "unit": false}]}` and exits 0 when it could run. A check counts as a unit of
work (tasks completed) unless it says `"unit": false` (a structural check, reported under correctness). The fixture's sha256
goes into every record. `trivial` has two independent files and exists only to test the harness.

## Fake agents

`fake_agent.py` reads a JSON script (the fixture's `fake.json`, or `--fake-script`) and performs file edits, git and klatch
commands from it, printing the same stream-json a real session prints. A step can be limited to some arms, can fail
(`exit`, a failing command), `hang` until the wall cap kills it, or report spend (`cost`). See the file's docstring for the
steps.

## Not computed yet

Fields the spec lists that need a scenario's unit-to-path map or the store export are written as `null` (never omitted):
`duplicate_work`, `claim_conflicts`, `overwrites`, `store_export`. `coordination_context_tokens` is an estimate (characters
divided by 4) because the standard library has no tokenizer; the record says so in `notes`. `messages` counts klatch posts
(`post`, `dm`, `ask`, `answer`) and `SendMessage` calls from the stream. A run found invalid (a harness fault) is kept with
`valid: false`; the rerun the spec asks for is made by hand with a new suite id until a scenario needs it automatic.

Tests: `python3 -m unittest discover -s experiments/coordbench/tests`. The K-arm tests need `KLATCH_BIN` (a built klatch,
`cargo build -p klatch --locked`) and are skipped with a message when it is unset.

Related: `experiments/variance/repeat.py` (the repeat harness this builds on), `proto/klatch-bench` (the scripted bench),
`experiments/dm-adoption` (the adoption runs).
