# Coordination bench

Does a team of agents coordinate better with klatch than without it? This directory holds the benchmark that answers it
(epic #1362). The rules are in [SPEC.md](SPEC.md): arms, scenarios S1 to S10, metrics, statistics, results schema,
conditions, thresholds (empty until the M0 baseline), milestones and threats to validity.

Status: the harness, the results ledger and the report generator exist (#1364) and run with fake agents. The real-agent
driver is built and its command and stream parser are unit tested, but no real run has been made: that needs the human's
written budget approval (SPEC section 8). Scenario fixtures other than the trivial test fixture are the scenario tickets'
work (#1366 to #1380), and the arm prompts stay DRAFT until #1365 reviews them.

- `SPEC.md`: the specification.
- `arms/`: the agent prompts for each arm. DRAFT until the B1 ticket (#1365) has them reviewed by someone who did not
  write them; the klatch arms must not get a better prompt than the baselines.
- `run.py`: runs one scenario against one or more arms, N times each (see Usage).
- `report.py`, `compare.py`: generate REPORT.md from a results directory; compare two results directories.
- `stats.py`: Fisher exact, Mann-Whitney U, Hodges-Lehmann, seeded bootstrap interval, Wilson, Newcombe and Holm, standard
  library only, tested against values from the literature.
- `streams.py`: parses the stream-json of an agent (turns, tokens, cost, tool calls, messages, conflicts).
- `drivers.py`, `fake_agent.py`: the agent driver interface with the `fake` (scripted, no model) and `claude` (headless
  `claude -p`) drivers.
- `fixture.py`, `fixtures/`, `checks/`: the fixture interface, the test fixture `trivial` and F1 `tallykit` (see Scenario S1).
- `scenario.py`, `scenarios/`: scenario files (fixture, arms, N, caps, unit-to-path map, metric definitions) and the git
  history metrics that need the map.
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

## Scenario S1: four agents, four tickets (`scenarios/s1.json`, fixture `tallykit`)

F1 `tallykit` is a standard library Python package of eight independent modules (units, tokenizer, dates, checksum, slug,
roman, rle, matrix), each a stub raising `NotImplementedError` with a docstring contract and a weak visible test. Four
tickets (agents `a` to `d`) implement two modules each, and every ticket also adds its lines at the end of the shared
registry `tallykit/__init__.py` and `CHANGELOG.md`, the only collisions, so a real merge conflict is possible. The spec
names a 10-module package with a `registry.py`; this fixture uses eight modules and `__init__.py` as the registry, as the
ticket asks (the spec sentence is a description, not a contract). `fixtures/tallykit/template/` is what `setup.sh` copies into a
checkout; `reference/` (the solution) is read only by tests and fake agents and is never copied.

The hidden checker `checks/tallykit/check.sh CHECKOUT` loads each module by file and runs seeded property tests against
independent oracles, then prints `{"checks": [...], "units_completed", "units_total", "merge_state", "correct", "per_agent"}`:
eight unit checks (the modules) and structural checks `registry` (one import line per module, every function exported),
`changelog` (exactly the four ticket lines), `no-conflict-markers` and `visible-tests`; `merge_state` is `clean`,
`conflicts-left` or `tests-broken`; `per_agent` is commits and modules touched by git author (each agent commits under its
own name; the harness sets that). `scenarios/s1.json` holds arms B0 B1 K1 K2 K3 (K3 not at M0), N 10 per arm at M0 and 3 for
the quick subset, caps (6 USD a run, 100 USD for the M0 suite on Haiku, from SPEC 8.1) and the metric definitions.

For a fixture with a scenario the harness now fills two fields it used to leave null. `duplicate_work` is the number of
modules changed by a non-merge commit of more than one agent; `overwrites` is the number of registry or changelog lines
added by some commit in the origin or any agent clone that are absent from the final main. The detail (which modules, which
lost lines and who wrote them, edits to a module the agent does not own, merge state, per-agent contribution) is in the
record's `scenario_metrics`. Not yet computed: `claim_conflicts` and `store_export` (the store, K arms). Fake scripts for
S1 use the extra steps `copy_from`, `append` and `sync` with `resolve` (union or ours); tests in `tests/test_tallykit_checker.py`
(the checker on the reference and on eight broken variants) and `tests/test_s1_scenario.py` (the metrics through the harness).

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
