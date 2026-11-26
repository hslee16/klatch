#!/usr/bin/env python3
"""Run one scenario against one or more arms, N times each, and keep every run (#1364, SPEC sections 4, 7, 8).

    run.py --fixture trivial --arm B0,B1,K1,K2 --runs 3 --driver fake --suite-id m0-try --klatch-bin PATH
    run.py --fixture trivial --arm B0,K2 --runs 10 --dry-run          # the plan and the cost range, nothing runs

Each run gets its own scratch directory under a temp root: an empty KLATCH_HOME and KLATCH_KEY_DIR (mode 700), a HOME of
its own, a bare `origin` made for the run and one fresh clone per agent. It never reads or writes ~/.klatch, the user's
Claude settings or the Keychain, and it refuses to start when a store path resolves under the live directories or outside
the temp root. The klatch CLI is reached only through the path given by --klatch-bin; no credential is ever passed to
it or to a fake agent. The only credential that exists is the model API key, passed by the claude driver alone, from the
harness environment, never printed and never written.

Arms are interleaved in randomised blocks (one run per arm per block, SPEC 6.5). The hidden checker runs on a clone of
the final origin after the agents exit (and every --poll-s seconds while they run, for time_to_first_pass). A per-run
spend cap and wall cap kill the team (outcomes spend-cap and timeout); a suite cap stops the suite and is recorded.
Records go to results/<suite-id>/<scenario>/<arm>/<run-id>.json, one line per run to results/LEDGER.md, stream logs to
--logs-dir outside the repository.

Exit codes: 0 every run ok, 1 some run was not ok or not valid, 2 refused (usage, guard, missing approval), 3 suite cap hit.
Real agents (--driver claude) cost money: they are refused unless --budget-approved-in names a valid budget approval file (JSON
outside the repository, see approval.py: approver, caps, models, expiry), whose numbers become the caps if lower and whose
approver, scope and path are recorded in the ledger header. This file is tested only with the fake driver.
"""
import json
import os
import random
import shutil
import signal
import sys
import tempfile
import threading
import time

import fixture as fixture_mod
import drivers
import isolation
import approval as approval_mod
import roles as roles_mod
import schema
import storeexport
# the names below were defined here before the split (#1455); tests and callers still reach them as run.NAME
from runcli import DEFAULT_ARMS, DEFAULT_RUN_USD, DEFAULT_SUITE_USD, dry_run, estimate, load_cost_model, model_ratio, parse_args, resolve_caps  # noqa: F401
from runenv import GuardError, commit_of, git, guard_paths, inside, kill_group, klatch_facts, minimal_env, now_iso, sha256_file  # noqa: F401
from runexec import RunExecution, RunSpot
from runprompt import TASK_BOARD, build_prompt  # noqa: F401
import metricver
import runidentity

HERE = os.path.dirname(os.path.abspath(__file__))


def plan_runs(arms, runs, seed):
    """Interleaved blocks: block b holds one run of each arm in an order drawn from the seed (SPEC 6.5)."""
    rng = random.Random(seed)
    plan = []
    for b in range(1, runs + 1):
        order = list(arms)
        rng.shuffle(order)
        plan += [(b, a) for a in order]
    return plan


class Suite:
    def __init__(self, args, fx, driver, plan):
        self.args, self.fx, self.driver, self.plan = args, fx, driver, plan
        self.spent = 0.0
        self.executed = 0
        self.failed = False
        self.cap_hit = None
        self.results_root = os.path.join(args.results_dir, args.suite_id)
        self.tmp_root = os.path.realpath(args.tmp_root)
        self.logs_root = args.logs_dir

    def run_one(self, arm, block, seed, run_id):
        started = now_iso()
        t_start = time.monotonic()
        scratch = tempfile.mkdtemp(prefix="cb-%s-" % run_id[-12:], dir=self.tmp_root)
        os.chmod(scratch, 0o700)
        isolation.write_owner(scratch)
        # while agents run the logs are in a directory with a random name (hidden); it takes the name logs_root/run_id afterwards
        tok = isolation.token()
        log_dir = os.path.join(self.logs_root, tok)
        os.makedirs(log_dir)
        marker = isolation.live_marker(self.logs_root, tok)
        rec = self.skeleton(run_id, arm, seed, started)
        notes, final = [], os.path.join(self.logs_root, run_id)
        published = False
        try:
            try:
                self.execute(rec, RunSpot(arm, seed, scratch, log_dir), notes)
            except (GuardError, fixture_mod.FixtureError, RuntimeError, OSError) as e:
                rec["outcome"], rec["valid"] = "error", False
                notes.append("setup failed: %s" % e)
            finally:
                isolation.restore(os.path.join(scratch, "agents"))  # hidden while agents ran (isolation.hide)
                isolation.restore(log_dir)
                shutil.rmtree(scratch, ignore_errors=True)
            rec["end"] = now_iso()
            rec["wall_s"] = round(time.monotonic() - t_start, 3) if rec["wall_s"] is None else rec["wall_s"]
            rec["notes"] = "; ".join(notes)
            rec = self.publish_logs(log_dir, final, rec, tok)
            published = True
            return rec
        finally:  # a SIGTERM or SIGINT ends the run here: the logs keep their public name, nothing random is left behind
            if not published and os.path.isdir(log_dir):
                self.publish_logs(log_dir, final, dict(rec, notes="interrupted"), tok)
            if os.path.exists(marker):
                os.unlink(marker)

    def publish_logs(self, work, final, rec, tok):
        """Rename the log directory to its public name (an older directory of the same run id is replaced) and make the
        record say so. The directory is hidden again when another process has a run in progress under this logs root: its
        agents could list this one (chmod u+rx to read it once that run is over)."""
        shutil.rmtree(final, ignore_errors=True)
        os.rename(work, final)
        rec = json.loads(json.dumps(rec).replace(json.dumps(work)[1:-1], json.dumps(final)[1:-1]))
        if isolation.other_live(self.logs_root, tok):
            isolation.hide(final)
            rec["notes"] = "; ".join(x for x in (rec["notes"], "log directory left unlistable (mode 0300): another run is live under this "
                                                 "logs root; chmod u+rx %s when it is over" % final) if x)
        return rec

    def skeleton(self, run_id, arm, seed, started):
        a = self.args
        sha, schema_no, version_output = self.klatch_facts
        return {
            "schema": schema.SCHEMA_VERSION, "run_id": run_id, "milestone": a.suite_id, "scenario": self.fx.name,
            "topology": self.fx.topology, "arm": arm, "arm_text_version": a.arm_text_version,
            "metric_version": metricver.METRIC_VERSION,
            **runidentity.identity(a, self.fx, arm, seed), "seed": seed, "valid": True,
            "supersedes": None,
            "models": {"agent": a.model or self.driver.name, "roles": a.roles_model if roles_mod.is_role_arm(arm) else None,
                       "resolved_ids": []},
            "claude_code_version": self.driver_version, "klatch": {"commit": self.commit, "binary_sha256": sha,
                                                                    "store_schema": schema_no, "version_output": version_output,
                                                                    "bench_commit": self.bench_commit},
            "temperature": "default",
            "caps": {"per_run_usd": a.max_run_usd, "suite_usd": a.max_suite_usd, "wall_s": a.wall_s,
                     "max_turns_per_agent": a.max_turns},
            "start": started, "end": None, "wall_s": None, "time_to_first_pass_s": None, "outcome": "error",
            "spend": {"usd": 0.0, "estimated": False}, "turns": {"total": 0, "per_agent": {}},
            "tokens": {"input": 0, "output": 0, "cache_read": 0, "cache_creation": 0},
            "tool_calls": {"total": 0, "by_tool": {}},
            "metrics": {"units_total": 0, "units_completed": 0, "duplicate_work": None, "git_conflicts": None,
                        "claim_conflicts": None, "overwrites": None, "messages": None, "human_interventions": None,
                        "coordination_context_tokens": None, "idle_turns": None},
            "outcome_checks": [], "roles": None, "isolation": None, "messaging_verified": None,
            "artifacts": {"streams": None, "store_export": None, "checker_report": None}, "notes": "",
        }

    def execute(self, rec, spot, notes):
        """One run's execution (runexec.RunExecution); git is looked up here so that run.git can be patched."""
        RunExecution(self, git, rec, spot, notes).execute()

    def write_record(self, rec):
        d = os.path.join(self.results_root, rec["scenario"], rec["arm"])
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, rec["run_id"] + ".json"), "w") as f:
            json.dump(rec, f, indent=2, sort_keys=True)
            f.write("\n")
        ledger = os.path.join(self.args.results_dir, "LEDGER.md")
        with open(ledger, "a") as f:
            f.write("%s | %s | %s | %s | %s | %.4f\n" % (rec["run_id"], rec["scenario"], rec["arm"], rec["outcome"],
                                                        str(rec["valid"]).lower(), rec["spend"]["usd"]))

    def write_meta(self, started, ended=None, not_run=()):
        a = self.args
        meta = {"suite_id": a.suite_id, "fixture": self.fx.name, "arms": a.arm_list, "runs_per_arm": a.runs,
                "seed": a.seed, "driver": self.driver.name, "model": a.model,
                "caps": {"per_run_usd": a.max_run_usd, "suite_usd": a.max_suite_usd, "wall_s": a.wall_s},
                "budget_approved_in": a.budget_approved_in, "budget_approval": a.approval, "planned": len(self.plan), "executed": self.executed,
                "spent_usd": round(self.spent, 6), "cap_hit": self.cap_hit, "not_run": list(not_run),
                "started": started, "ended": ended}
        os.makedirs(self.results_root, exist_ok=True)
        with open(os.path.join(self.results_root, "suite.json"), "w") as f:
            json.dump(meta, f, indent=2, sort_keys=True)
            f.write("\n")

    def go(self):
        a = self.args
        os.makedirs(a.results_dir, exist_ok=True)
        os.makedirs(self.logs_root, exist_ok=True)
        os.makedirs(self.tmp_root, exist_ok=True)
        isolation.sweep_stale(self.tmp_root)  # what a run that was killed left behind (mode 0300)
        isolation.sweep_logs(self.logs_root)
        old = self.install_signal_handlers()
        try:
            return self.go_hidden()
        finally:
            for sig, handler in old.items():
                signal.signal(sig, handler)

    @staticmethod
    def install_signal_handlers():
        """SIGTERM and SIGINT restore the modes of every directory this process hid and then end the suite through the
        finally blocks (agents killed, scratch removed). Only in the main thread; a SIGKILL is left to sweep_stale."""
        if threading.current_thread() is not threading.main_thread():
            return {}

        def handler(signum, _frame):
            isolation.restore_all()
            raise SystemExit(128 + signum)
        return {sig: signal.signal(sig, handler) for sig in (signal.SIGTERM, signal.SIGINT)}

    def go_hidden(self):
        a = self.args
        probe_home = tempfile.mkdtemp(prefix="cb-probe-", dir=self.tmp_root)
        try:
            self.klatch_facts = klatch_facts(a.klatch_bin, minimal_env(os.environ, probe_home))
        finally:
            shutil.rmtree(probe_home, ignore_errors=True)
        self.commit = a.klatch_commit  # only what the caller says: the bench repository's HEAD is not the binary's commit (#1547)
        self.bench_commit = commit_of(HERE)
        self.driver_version = self.driver.version()
        started = now_iso()
        with open(os.path.join(a.results_dir, "LEDGER.md"), "a") as f:
            f.write("# suite %s started %s, driver %s, caps per run %.2f suite %.2f USD, budget approval: %s\n"
                    % (a.suite_id, started, self.driver.name, a.max_run_usd, a.max_suite_usd,
                       ("%s, approved by %s on %s, scope: %s" % (a.budget_approved_in, a.approval["approved_by"],
                                                                a.approval["approved_on"], a.approval["scope"]))
                       if a.approval else (a.budget_approved_in or "none (fake agents only)")))
        self.write_meta(started)
        not_run = []
        for i, (block, arm) in enumerate(self.plan):
            if self.cap_hit:
                not_run.append("%s-%s-r%02d" % (self.fx.name, arm, block))
                continue
            if self.spent + a.max_run_usd > a.max_suite_usd:
                self.cap_hit = {"reason": "suite-cap", "spent_usd": round(self.spent, 6),
                                "suite_usd": a.max_suite_usd, "before_run": "%s-%s-r%02d" % (self.fx.name, arm, block)}
                print("suite spend cap: %.2f of %.2f USD spent; the next run could cost %.2f, stopping"
                      % (self.spent, a.max_suite_usd, a.max_run_usd))
                not_run.append("%s-%s-r%02d" % (self.fx.name, arm, block))
                continue
            run_id = "%s-%s-%s-r%02d" % (a.suite_id, self.fx.name.lower(), arm.lower(), block)
            rec = self.run_one(arm, block, a.seed + block, run_id)
            self.write_record(rec)
            self.executed += 1
            self.spent += rec["spend"]["usd"]
            if rec["outcome"] != "ok" or not rec["valid"]:
                self.failed = True
            print("%s: %s, units %d/%d, %.4f USD, %.1f s%s" % (
                run_id, rec["outcome"], rec["metrics"]["units_completed"], rec["metrics"]["units_total"],
                rec["spend"]["usd"], rec["wall_s"], "" if rec["valid"] else " (INVALID: %s)" % rec["notes"]))
            if rec["outcome"] != "ok" and rec["valid"] and rec["notes"]:
                print("%s: why: %s" % (run_id, rec["notes"]))  # a crash's reason, not only the summary line (#1639)
            sys.stdout.flush()
        self.write_meta(started, now_iso(), not_run)
        print("done: %d of %d runs, %.4f USD, results in %s" % (self.executed, len(self.plan), self.spent, self.results_root))
        return 3 if self.cap_hit else (1 if self.failed else 0)


def _absolute(path):
    """The path made absolute when it is relative (an absolute one is kept as given); None and "" stay as they are."""
    return os.path.abspath(path) if path and not os.path.isabs(path) else path


def _refuse(why):
    print("run.py: %s" % why, file=sys.stderr)
    return 2


def _unsafe_root(a):
    """Why --logs-dir or --tmp-root is refused (the harness makes directories in them and hides some), or None."""
    strict = a.driver == "claude" and a.isolation == "sandbox"
    problems = [isolation.validate_dir(v, w, strict) for v, w in ((a.logs_dir, "--logs-dir"), (a.tmp_root, "--tmp-root"))]
    return next((p for p in problems if p), None)


def _needs_klatch_bin(a):
    return any(x.startswith("K") for x in a.arm_list) and not a.klatch_bin and not a.dry_run


def _usage_problem(a):
    """Why the arguments are refused (exit 2), or None."""
    if a.runs < 1 or not a.arm_list:
        return "need at least one arm and one run"
    missing = [arm for arm in a.arm_list if not os.path.exists(os.path.join(a.arms_dir, arm + ".md"))]
    if missing:
        return "no arm prompt %s.md in %s" % (missing[0], a.arms_dir)
    if _needs_klatch_bin(a):
        return "the K arms need --klatch-bin (the absolute path of the klatch binary)"
    return None


def _claude_driver(a, fx):
    """(the real driver, None), or (None, why it is refused): real agents cost money and need the human's approval."""
    if not a.budget_approved_in:
        return None, ("real agents cost money; --budget-approved-in must name the budget approval file the human "
                      "made (SPEC section 8, approval.py). Without it only --driver fake runs.")
    if not a.model:
        return None, "--driver claude needs --model (a pinned model id)"
    if getattr(a, "committed_facts", False):
        return None, "--committed-facts is for the fake driver: real agents get hidden facts drawn from the run's seed (#1531)"
    try:
        a.max_run_usd, a.max_suite_usd = resolve_caps(a)
    except approval_mod.ApprovalError as e:
        return None, str(e)
    return drivers.ClaudeDriver(a.claude_bin, a.model, a.max_turns, a.max_run_usd / len(fx.agents), a.klatch_bin), None


def _fake_driver(a, fx):
    script = a.fake_script or fx.fake_script
    if not script:
        return None, "the fixture has no fake script; give --fake-script"
    return drivers.FakeDriver(os.path.abspath(script), a.klatch_bin, a.room), None


def main(argv=None):
    a = parse_args(argv if argv is not None else sys.argv[1:])
    a.arm_list = [x for x in a.arm.split(",") if x]
    problem = _usage_problem(a)
    if problem:
        return _refuse(problem)
    a.klatch_bin = _absolute(a.klatch_bin)
    problem = _unsafe_root(a)
    if problem:
        return _refuse(problem)
    try:
        fx = fixture_mod.Fixture(a.fixture, a.fixtures_dir, a.checks_dir, variant=a.variant)
    except (fixture_mod.FixtureError, KeyError) as e:
        return _refuse(e)
    plan = plan_runs(a.arm_list, a.runs, a.seed)
    if a.dry_run:
        return dry_run(a, fx, plan)
    a.tmp_root = os.path.realpath(a.tmp_root)
    try:
        guard_paths(os.path.join(a.tmp_root, "x"), os.path.join(a.tmp_root, "y"), a.tmp_root)
    except GuardError as e:
        return _refuse(e)
    staged = _stage_klatch(a)
    try:
        return _run_suite(a, fx, plan)
    finally:
        if staged:
            shutil.rmtree(staged, ignore_errors=True)


def _stage_klatch(a):
    """The agents get a copy of the binary (its directory is returned): no prompt names a path inside the repository (#1519)."""
    if not (a.klatch_bin and os.path.isfile(a.klatch_bin)):
        return None
    os.makedirs(a.tmp_root, exist_ok=True)
    a.bench_bin = a.bench_bin or storeexport.find_tool(a.klatch_bin, os.environ)  # found beside the original, before the copy
    a.klatch_bin, staged = isolation.stage_binary(a.klatch_bin, a.tmp_root)
    return staged


def isolation_problem(a, driver):
    """Why a real run cannot use the sandbox settings (Claude Code too old or unknown), or None."""
    if a.isolation != "sandbox" or driver.name == "fake":
        return None
    return isolation.version_problem(driver.version())


def _run_suite(a, fx, plan):
    driver, problem = (_claude_driver if a.driver == "claude" else _fake_driver)(a, fx)
    problem = problem or isolation_problem(a, driver)
    if problem:
        return _refuse(problem)
    return Suite(a, fx, driver, plan).go()


if __name__ == "__main__":
    sys.exit(main())
