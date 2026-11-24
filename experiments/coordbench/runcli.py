"""The command line of run.py and the plan it prints (split out of run.py, #1455): the arguments, the spend caps and
the dry run with its cost estimate. Standard library only; nothing here starts an agent."""
import argparse
import json
import os
import tempfile

import approval as approval_mod

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_ARMS = "B0,B1,K1,K2"
DEFAULT_RUN_USD = 3.0
DEFAULT_SUITE_USD = 150.0


def load_cost_model():
    with open(os.path.join(HERE, "cost_model.json")) as f:
        return json.load(f)


def model_ratio(model, cm):
    m = (model or "").lower()
    for fam in ("opus", "sonnet", "haiku"):
        if fam in m:
            return cm["model_ratio"][fam]
    return 1.0


def estimate(fixture, n_runs, model):
    cm = load_cost_model()
    per = fixture.spec.get("est_cost_usd_haiku")
    if per is None:
        per = cm["per_run_haiku_usd"].get(fixture.name.lower())
    if per is None:
        return None
    ratio = model_ratio(model, cm)
    base = per * ratio * n_runs
    lo, hi = cm["range_factors"]
    return {"per_run": per * ratio, "ratio": ratio, "low": base * lo, "high": base * hi, "factors": (lo, hi), "cm": cm}


def parse_args(argv):
    p = argparse.ArgumentParser(description="Run a coordination bench scenario against arms, N times each (#1364).")
    p.add_argument("--fixture", required=True)
    p.add_argument("--variant", help="a variant of the fixture (S9 history: join, the default, or resume); a variant other than "
                   "join is a separate scenario name in the records")
    p.add_argument("--arm", default=DEFAULT_ARMS, help="comma-separated arms (default %s)" % DEFAULT_ARMS)
    p.add_argument("--runs", type=int, default=3, help="runs per arm (blocks)")
    p.add_argument("--driver", choices=("fake", "claude"), default="fake")
    p.add_argument("--suite-id", required=True, help="milestone: m0, epic-<ticket> or release-<version>")
    p.add_argument("--results-dir", default=os.path.join(HERE, "results"))
    p.add_argument("--logs-dir", default=os.path.join(tempfile.gettempdir(), "coordbench-logs"),
                   help="where stream logs and prompts are kept (outside the repository)")
    p.add_argument("--tmp-root", default=tempfile.gettempdir())
    p.add_argument("--fixtures-dir", default=os.path.join(HERE, "fixtures"))
    p.add_argument("--checks-dir", default=os.path.join(HERE, "checks"))
    p.add_argument("--scenarios-dir", default=os.path.join(HERE, "scenarios"))
    p.add_argument("--arms-dir", default=os.path.join(HERE, "arms"))
    p.add_argument("--arm-text-version", default="draft-1")
    p.add_argument("--klatch-bin", help="absolute path of the klatch binary (needed by K arms)")
    p.add_argument("--klatch-commit", help="the klatch commit (default: git HEAD of this repository)")
    p.add_argument("--bench-bin", help="the klatch-bench tool that exports the K arms' run store as counts (default: "
                   "$KLATCH_BENCH_BIN, else klatch-bench beside the klatch binary; without it the export is skipped with a note)")
    p.add_argument("--isolation", choices=("sandbox", "layout"), default="sandbox",
                   help="how a real agent is kept to its own directories (#1519): sandbox (the default: Claude Code's OS sandbox "
                   "for Bash and its children plus file-tool deny rules, in the --settings of every agent) or layout (only "
                   "the unlistable directories and the post-run scan; use it when the sandbox breaks a K arm). Recorded in "
                   "every record. The scan runs in both.")
    p.add_argument("--room", default="lobby")
    p.add_argument("--seed", type=int, default=1000)
    p.add_argument("--model", help="the agent model id (required with --driver claude)")
    p.add_argument("--roles-model", help="the model of the role members of K3 and B1-roles (default: the scenario file's, else --model)")
    p.add_argument("--roles", help="the role members to start in K3 and B1-roles, comma separated (default: the scenario "
                   "file's, else integrator,supervisor)")
    p.add_argument("--role-usd", type=float, help="the spend cap of each role member (default: the scenario file's, else the run "
                   "cap spread over every member)")
    p.add_argument("--roles-grace-s", type=float, help="how long the role members may run on after every worker has ended "
                   "(default: the scenario file's, else 120)")
    p.add_argument("--claude-bin", default="claude")
    p.add_argument("--fake-script", help="a fake-agent script (default: the fixture's)")
    p.add_argument("--max-run-usd", type=float, default=None, help="per-run spend cap (default %g; with an approval file "
                   "the file's cap)" % DEFAULT_RUN_USD)
    p.add_argument("--max-suite-usd", type=float, default=None, help="suite spend cap (default %g; with an approval file "
                   "the file's budget)" % DEFAULT_SUITE_USD)
    p.add_argument("--wall-s", type=float, default=1800)
    p.add_argument("--max-turns", type=int, default=60, help="turn cap per agent")
    p.add_argument("--poll-s", type=float, default=60.0, help="how often the hidden checker is polled while agents run")
    p.add_argument("--snapshot-s", type=float, default=5.0, help="T-shared only (#1427): how often the watcher reads the shared "
                   "file (it also reads it whenever an agent exits); the resolution of the lost-edit measure")
    p.add_argument("--stagger-s", type=float, default=0.0, help="launch stagger of agents, drawn from the seed")
    p.add_argument("--budget-approved-in", metavar="FILE", help="the budget approval file (JSON, outside the repository; "
                   "see approval.py and README.md); required with --driver claude")
    p.add_argument("--dry-run", action="store_true", help="print the plan and the cost range; run nothing, write nothing")
    a = p.parse_args(argv)
    a.cli_run_cap, a.cli_suite_cap = a.max_run_usd, a.max_suite_usd  # None when not given on the command line
    a.max_run_usd = DEFAULT_RUN_USD if a.max_run_usd is None else a.max_run_usd
    a.max_suite_usd = DEFAULT_SUITE_USD if a.max_suite_usd is None else a.max_suite_usd
    a.approval = None
    return a


def resolve_caps(a):
    """(per-run cap, suite cap) in USD. A real run takes them from the approval file (validated here, see approval.py;
    ApprovalError when it refuses); a fake run from the command line or the defaults."""
    if a.driver == "claude":
        a.approval = approval_mod.load(a.budget_approved_in, a.model, a.roles_model, a.cli_run_cap, a.cli_suite_cap)
        return a.approval["per_run_usd"], a.approval["suite_usd"]
    return a.max_run_usd, a.max_suite_usd


def dry_run(a, fx, plan):
    est = estimate(fx, len(plan), a.model)
    print("DRY RUN: nothing is started and nothing is written.")
    print("suite %s, fixture %s, driver %s, model %s" % (a.suite_id, fx.name, a.driver, a.model or "(none)"))
    print("arms %s, %d blocks, %d runs, %d agents per run" % (",".join(a.arm_list), a.runs, len(plan), len(fx.agents)))
    print("caps: per run %.2f USD, suite %.2f USD, wall %d s, %d turns per agent" % (
        a.max_run_usd, a.max_suite_usd, a.wall_s, a.max_turns))
    for b in range(1, a.runs + 1):
        print("  block %d: %s" % (b, " ".join(arm for blk, arm in plan if blk == b)))
    if est is None:
        print("estimated cost: no planning figure for fixture %s (add est_cost_usd_haiku to its fixture.json)" % fx.name)
    else:
        print("estimated cost: %.2f to %.2f USD for %d runs (assumed %.4f USD a run at model ratio %g, range %gx to %gx; "
              "SPEC 8.1 planning figures, replaced by a measured pilot)" % (
                  est["low"], est["high"], len(plan), est["per_run"], est["ratio"], est["factors"][0], est["factors"][1]))
        print("proposed caps: per run %.2f USD, suite %.2f USD (SPEC 8.1: 3x the assumed run cost, 1.25x the estimate)" % (
            est["per_run"] * est["cm"]["per_run_cap_factor"],
            est["per_run"] * len(plan) * est["cm"]["suite_cap_factor"]))
    print("a fake driver spends nothing; a real run needs the human's budget approval (--budget-approved-in)")
    return 0
