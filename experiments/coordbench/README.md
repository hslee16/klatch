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
- `fixture.py`, `fixtures/`, `checks/`: the fixture interface, the test fixture `trivial`, F1 `tallykit` (see Scenario S1) and F2 `onefile` (see Scenario S2) and F3 `chain` (see Scenario S3).
- `chain.py`: the stream metrics of S3 (a blocked agent's idle and wasted turns, the handoff claim record).
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

A real run (not made yet) adds `--driver claude --model MODEL_ID --claude-bin PATH --budget-approved-in FILE`, and
optionally `--max-run-usd N --max-suite-usd N`. FILE is the budget approval (SPEC section 8; `approval.py`): a JSON file
the coordinator makes at run time, outside the repository (never commit one), for example

    {"approved_by": "NAME", "approved_on": "2026-11-23", "suite_budget_usd": 2.0, "per_run_cap_usd": 0.5,
     "scope": "The B1 pilot, one run per arm on the trivial fixture.", "models": ["MODEL_ID"], "expires_on": "2026-11-30"}

All seven fields are required. The harness refuses (exit 2, nothing written) when the file is missing, malformed or
expired, when `--model` or `--roles-model` is not in `models`, or when a cap given on the command line is above the
file's; caps not given become the file's numbers, and caps given below them stand. The ledger header records the file's
path, approver, date and scope. The model API key (`ANTHROPIC_API_KEY`) is read from the
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

## Scenario S2: three agents, one file (`scenarios/s2.json`, fixture `onefile`)

F2 `onefile` is a standard library settings renderer whose whole logic is `onefile/core.py` (about 120 lines with the three
changes in, 85 before). Three agents (`a` to `c`) are told to change it at once with no ordering: A adds the `timeout`
option and `parse_timeout`; B adds the `retries` option and `backoff_delays`; C changes `render_header` and adds the `label`
option. Every ticket adds a row to the shared option table `OPTIONS` at the same place, A and B add a kind to the shared
helper `format_value` at the same place, and A and B append a function at the end of the file, so concurrent edits conflict at
merge, and a stale copy or a force push silently drops a colleague's work. The header prints the table size, so the changes
must agree in the merged file. The visible tests cover the old behaviour only and pass after any subset of the changes.
`fixtures/onefile/reference/` (`core.py`, the full solution, and `edits.json`, each ticket's edit as fake-agent `replace` and
`append` steps) is read only by tests and fake agents and is never copied into a checkout.

The hidden checker `checks/onefile/check.sh CHECKOUT` loads `core.py` by file and tests each ticket alone against
independent oracles and seeded random inputs. It prints `{"checks", "units_completed", "units_total", "merge_state",
"correct", "lost_edits", "lost_table_entries", "per_agent"}`: three unit checks (`timeout`, `retries`, `header`) and structural
checks `table` (exactly the five rows), `integrated-render`, `imports`, `no-conflict-markers`, `visible-tests`; `merge_state`
is `clean`, `conflicts-left` or `tests-broken`; `lost_edits` are the tickets missing from main; `per_agent` is commits and
surviving lines of `core.py` (git blame) by author. `check.sh --file PATH` says which tickets one version of the file has.

Metrics (definitions in the scenario file). `units_completed` 0 to 3 is primary. `overwrites` is computed by the harness for a
scenario with `history_check`: every non-merge commit of the origin and of every agent's clone is tested with the checker's
per-ticket check on the file as it has it; a ticket that passes on some commit and fails on the final main is an overwrite
(edits committed anywhere and missing from the final main). `silent_overwrites` is those overwrites in a run where no agent saw
a git conflict; `overwrites_after_conflict` is those in a run where one did (a destructive resolution or an unresolved
conflict); `conflicts_resolved` is the run's git conflicts when it ended correct with no overwrite. The split is per run, so a
run with any conflict puts all its lost edits under `overwrites_after_conflict`. `git_conflicts` and `merge_state` are as in S1.
N is 20 per arm at M0 (3 for the quick subset); caps are SPEC 8.1's (1.80 USD a run, 60 USD for one topology on Haiku),
proposals the human changes before M0.

Gaps, stated plainly. Topology T-shared (one working directory used by all agents, SPEC's primary topology for the K arms) is
not built: `run.py` makes one clone per agent under the run's scratch, the arm prompts say "your own clone", and both drivers
take one working directory per agent, so only T-clone runs. T-shared needs its own run mode (one shared checkout), a prompt
variant that says so, and metrics that do not rely on commits (a final-file check, since agents may never commit); this is a
follow-up. `claim_conflicts` is null (it needs the K arms' store export) and `time_to_recover_s` is null (the stream parser
counts conflicts but keeps no time of the first one). Fake scripts use the new `replace` step (an edit of the file as it is
now, failing when the text is absent) next to `copy_from`, `append`, `sync` and `git`; tests are in
`tests/test_onefile_checker.py` (the checker on the reference and on eleven variants) and `tests/test_s2_scenario.py` (the
metrics through the harness with fake agents that cooperate, collide and resolve, collide and leave it, push a stale copy,
resolve destructively and give up).

## Fake agents

`fake_agent.py` reads a JSON script (the fixture's `fake.json`, or `--fake-script`) and performs file edits, git and klatch
commands from it, printing the same stream-json a real session prints. A step can be limited to some arms, can fail
(`exit`, a failing command), `hang` until the wall cap kills it, or report spend (`cost`). See the file's docstring for the
steps.

## Not computed yet

Fields the spec lists that need a scenario's unit-to-path map or the store export are written as `null` (never omitted):
`duplicate_work`, `claim_conflicts`, `overwrites`, `store_export` (S1 and S2 fill `duplicate_work` and `overwrites`; S2 leaves `claim_conflicts` and `time_to_recover_s` null). `coordination_context_tokens` is an estimate (characters
divided by 4) because the standard library has no tokenizer; the record says so in `notes`. `messages` counts klatch posts
(`post`, `dm`, `ask`, `answer`) and `SendMessage` calls from the stream. A run found invalid (a harness fault) is kept with
`valid: false`; the rerun the spec asks for is made by hand with a new suite id until a scenario needs it automatic.

Tests: `python3 -m unittest discover -s experiments/coordbench/tests`. The K-arm tests need `KLATCH_BIN` (a built klatch,
`cargo build -p klatch --locked`) and are skipped with a message when it is unset.

Related: `experiments/variance/repeat.py` (the repeat harness this builds on), `proto/klatch-bench` (the scripted bench),
`experiments/dm-adoption` (the adoption runs).

## Scenario S3: a dependency chain and one handoff (`scenarios/s3.json`, fixture `chain`)

F3 `chain` (the spec calls it `pipeline`) is a standard library inventory reader. Three agents work one after another on four
steps: 1 the schema file (`chainkit/schema.json`, generated from `data/ruler.txt` by `tools/make_schema.py`), 2 the bin codes
(`chainkit/bins.py`), 3 the loader (`chainkit/loader.py`), 4 the report (`chainkit/report.py`). Agent `a` does step 1 and
writes `parse_bin` (returns `Bin(pfx, num)`), leaves `canonical_bin` unfinished, and is then reassigned and told to hand over
to `c`. Agent `b` cannot do step 3 until a's schema file is on main (its task text does not give the format). Agent `c`
finishes `canonical_bin` and does step 4, which needs b's loader and c's own bin codes. The one handoff carries a quirk: `a`'s
task text (and no other file an agent sees) says that bins with one legacy prefix were relocated, so their canonical code has
another prefix and the number plus an arbitrary offset. The prefixes and the offset come from a fixed seed (1368) in
`fixtures/chain/gen.py` and cannot be guessed; if a's knowledge does not cross the handoff, c's `canonical_bin` is right for
every bin but the legacy ones. `gen.py` makes the data, the ruler, a's task text, the fake script, the reference and the
checker's `expected.json` (`gen.py --check` says when a committed file is out of date; a test runs it).

The hidden checker `checks/chain/check.sh CHECKOUT` tests each step alone (the loader with the reference schema file in place,
the report with the reference loader, normaliser and schema, so one failure cannot hide behind another) and prints
`{"checks", "units_completed", "units_total", "steps_correct", "quirk_honoured", "lost_at_handoff", "order", "merge_state",
"correct"}`: four unit checks (`schema`, `bins`, `loader`, `report`; the loader must also work on a different column layout,
so it cannot have the columns written in) and structural checks `quirk` (every legacy record of the real data and seeded
cases), `handoff-state`, `integrated` (the whole pipeline on the final main, nothing substituted), `order-loader-after-schema`
and `order-report-after-loader` (the first commit of a step had its input in its tree), `no-conflict-markers`,
`visible-tests`. `lost_at_handoff` lists a's published pieces (the schema file, `parse_bin` with its `Bin(pfx, num)` result)
that met their contract in some commit and fail on the final main, as dropped or redone-differently; a piece that never met
its contract is a failed step, not lost work. The reference solution is `fixtures/chain/reference/` (never in a checkout);
`tests/test_chain_checker.py` runs the checker on it and on variants that must fail: the quirk ignored, a guessed offset, a
wrong prefix, `parse_bin` dropped or redone, the schema replaced or deleted, a report or loader committed before its input,
a loader with the columns written in, a loader returning tuples, a renamed report function, a miscounting report, conflict
markers and a failing visible test.

Metrics (definitions in the scenario file). Primary `steps_correct` (0 to 4); `quirk_honoured` (yes or no) and
`lost_at_handoff` are from the checker. The blocked agent's metrics are computed from its stream by `chain.py` (a turn is one
assistant message). The dependency is SEEN at the first turn whose tool result shows the file (a `git pull` diffstat, a
listing, its content) without saying it is missing; a message that only names the file does not count. `blocked_turns` are the
turns before that; `idle_turns` are blocked turns that only wait or look, meaning every tool call is a poll (`sleep`,
`klatch wait`, `klatch read` or `history` or `board` or `claims` or `asks`, `git fetch` or `pull` or `log` or `status`, `ls`,
`test`, a read of `TASKS.md`, `ListAgents`, optionally piped through `head`, `tail`, `grep`, `wc`) or the turn has no tool call;
`guess_build_turns` are blocked turns that write the dependent step's file (a loader built before the schema exists);
`wasted_turns` is their sum. They are reported for `b` (waiting for the schema file) and `c` (waiting for the loader), per
agent, never summed. The harness also fills `overwrites` (a's `def parse_bin` and `Bin` lines absent from the final main) and
`duplicate_work` (a step changed by more than one agent) from git history. Time to completion is `time_to_first_pass_s` and
`wall_s`. Whether a claim moves with a handoff (#539) is recorded for the K arms as `handoff_claims`, not graded: from the
streams, the paths `a` claimed, released, released before its handoff message and still held when it stopped, what `c`
claimed and was refused, and whether `a` posted `checkpoint:` and `handoff:` messages. Whether klatch itself moved or freed a
claim or an ask needs the store, which `run.py` does not export yet, so `transferred_by_klatch` is null with a note (the same
gap as `claim_conflicts`). B arms have null `handoff_claims`. N is 10 per arm at M0 (3 for the quick subset); caps are SPEC
8.1's (2.40 USD a run, 40 USD for the M0 suite on Haiku), proposals the human changes before M0. The fake agent gained a `poll`
step (n polling turns without waiting) for the blocked-agent tests. No real run has been made.
