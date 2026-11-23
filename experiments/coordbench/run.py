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
import argparse
import datetime
import hashlib
import json
import os
import random
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time

import approval as approval_mod
import drivers
import fixture as fixture_mod
import asks as asks_mod
import scenario as scenario_mod
import schema
import streams

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_ARMS = "B0,B1,K1,K2"
DEFAULT_RUN_USD = 3.0
DEFAULT_SUITE_USD = 150.0
TASK_BOARD = ("# Task board\n\nOne line per piece of work: `- WHO | STATUS | FILES | TITLE`. STATUS is `working`, `done`, "
              "`blocked` or `handoff`. Add your line at the end, commit it and push it.\n")


class GuardError(Exception):
    pass


def now_iso():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def inside(path, root):
    path, root = os.path.realpath(path), os.path.realpath(root)
    return path == root or path.startswith(root.rstrip(os.sep) + os.sep)


def guard_paths(klatch_home, key_dir, tmp_root):
    """Refuse any store path that could be the live one (SPEC 8, Isolation). Raises GuardError."""
    home = os.path.expanduser("~")
    live = [os.path.join(home, ".klatch"), os.path.join(home, ".claude")]
    for root in [tmp_root]:
        for l in live:
            if inside(root, l):
                raise GuardError("the temp root %s is under %s" % (root, l))
    for p in (klatch_home, key_dir):
        for l in live:
            if inside(p, l):
                raise GuardError("refusing the live directory: %s resolves under %s" % (p, l))
        if not inside(p, tmp_root):
            raise GuardError("%s resolves outside the temp root %s" % (p, tmp_root))


def plan_runs(arms, runs, seed):
    """Interleaved blocks: block b holds one run of each arm in an order drawn from the seed (SPEC 6.5)."""
    rng = random.Random(seed)
    plan = []
    for b in range(1, runs + 1):
        order = list(arms)
        rng.shuffle(order)
        plan += [(b, a) for a in order]
    return plan


def build_prompt(arms_dir, arm, name, team, task, workdir, test_command, klatch_bin, room, max_turns, max_minutes,
                 restart_command=""):
    """The common prompt with the arm's coordination section, every placeholder filled, draft comments removed."""
    def load(fn):
        with open(os.path.join(arms_dir, fn)) as f:
            text = re.sub(r"<!--.*?-->", "", f.read(), flags=re.S)
        return text.strip() + "\n"
    common = re.sub(r"^# Common part[^\n]*\n", "", load("common.md")).lstrip()
    text = common.replace("{{COORDINATION_SECTION}}", load(arm + ".md").strip())
    values = {"NAME": name, "TEAM_SIZE": str(len(team)), "TEAM_NAMES": ", ".join(n for n in team if n != name),
              "WORKDIR": workdir, "TASK": task, "TEST_COMMAND": test_command, "KLATCH": klatch_bin or "",
              "ROOM": room, "MAX_TURNS": str(max_turns), "MAX_MINUTES": str(max_minutes),
              "RESTART_COMMAND": restart_command}
    for k, v in values.items():
        text = text.replace("{{%s}}" % k, v)
    left = re.findall(r"\{\{[A-Z_]+\}\}", text)
    if left:
        raise ValueError("unfilled placeholders in the %s prompt: %s" % (arm, ", ".join(sorted(set(left)))))
    return text


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


def minimal_env(base, scratch_home):
    """The environment of the harness's own helper processes (git, setup, checker, klatch --version): an allowlist,
    a private HOME, never a credential."""
    env = {k: base[k] for k in drivers.BASE_ENV_KEYS if k in base}
    env.update({"HOME": scratch_home, "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
                "GIT_TERMINAL_PROMPT": "0", "GIT_AUTHOR_NAME": "bench", "GIT_AUTHOR_EMAIL": "bench@bench.invalid",
                "GIT_COMMITTER_NAME": "bench", "GIT_COMMITTER_EMAIL": "bench@bench.invalid"})
    return env


def git(args, cwd, env):
    p = subprocess.run(["git"] + args, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    if p.returncode != 0:
        raise RuntimeError("git %s failed: %s" % (" ".join(args), p.stdout.strip()[-300:]))
    return p.stdout


def kill_group(proc):
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except OSError:
        pass


def klatch_facts(klatch_bin, env):
    """(binary sha256, store schema) of the klatch under test; (None, None) when no binary was given."""
    if not klatch_bin:
        return None, None
    schema_no = None
    try:
        p = subprocess.run([klatch_bin, "--version"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
                           env=env, timeout=30)
        m = re.search(r"store schema (\d+)", p.stdout)
        schema_no = int(m.group(1)) if m else None
    except (OSError, subprocess.SubprocessError):
        pass
    return sha256_file(klatch_bin), schema_no


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

    # one run -------------------------------------------------------------------------------------------------

    def run_one(self, arm, block, seed, run_id):
        started = now_iso()
        t_start = time.monotonic()
        scratch = tempfile.mkdtemp(prefix="cb-%s-" % run_id[-12:], dir=self.tmp_root)
        os.chmod(scratch, 0o700)
        log_dir = os.path.join(self.logs_root, run_id)
        os.makedirs(log_dir, exist_ok=True)
        rec = self.skeleton(run_id, arm, seed, started)
        notes = []
        try:
            self.execute(rec, arm, seed, scratch, log_dir, t_start, notes)
        except (GuardError, fixture_mod.FixtureError, RuntimeError, OSError) as e:
            rec["outcome"], rec["valid"] = "error", False
            notes.append("setup failed: %s" % e)
        finally:
            shutil.rmtree(scratch, ignore_errors=True)
        rec["end"] = now_iso()
        rec["wall_s"] = round(time.monotonic() - t_start, 3) if rec["wall_s"] is None else rec["wall_s"]
        rec["notes"] = "; ".join(notes)
        return rec

    def skeleton(self, run_id, arm, seed, started):
        a = self.args
        sha, schema_no = self.klatch_facts
        return {
            "schema": schema.SCHEMA_VERSION, "run_id": run_id, "milestone": a.suite_id, "scenario": self.fx.name,
            "topology": self.fx.topology, "arm": arm, "arm_text_version": a.arm_text_version,
            "fixture": {"name": self.fx.name, "sha256": self.fx.sha256()}, "seed": seed, "valid": True,
            "supersedes": None,
            "models": {"agent": a.model or self.driver.name, "roles": a.roles_model, "resolved_ids": []},
            "claude_code_version": self.driver_version, "klatch": {"commit": self.commit, "binary_sha256": sha,
                                                                    "store_schema": schema_no},
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
            "outcome_checks": [],
            "artifacts": {"streams": None, "store_export": None, "checker_report": None}, "notes": "",
        }

    def execute(self, rec, arm, seed, scratch, log_dir, t_start, notes):
        a, fx = self.args, self.fx
        home = os.path.join(scratch, "home")
        khome = os.path.join(scratch, "klatch-home")
        kkeys = os.path.join(scratch, "klatch-keys")
        for d in (home, khome, kkeys):
            os.makedirs(d)
            os.chmod(d, 0o700)
        guard_paths(khome, kkeys, self.tmp_root)
        henv = minimal_env(os.environ, home)
        # the repository: initial tree from the fixture, a bare origin made for this run
        seedtree = os.path.join(scratch, "seed")
        fx.materialise(seedtree, seed, henv)
        if arm == "B1":
            with open(os.path.join(seedtree, "TASKS.md"), "w") as f:
                f.write(TASK_BOARD + "".join(l + "\n" for l in (fx.seed_history or {}).get("board", [])))
        origin = os.path.join(scratch, "origin.git")
        git(["init", "-q", "--bare", "-b", "main", origin], scratch, henv)
        git(["init", "-q", "-b", "main"], seedtree, henv)
        git(["add", "-A"], seedtree, henv)
        git(["commit", "-q", "-m", "initial"], seedtree, henv)
        git(["push", "-q", origin, "HEAD:main"], seedtree, henv)
        shutil.rmtree(seedtree)
        # agents
        procs, states, logs, offs, partial = {}, {}, {}, {}, {}
        rng = random.Random(seed)
        team = fx.names
        clones = {}
        for ag in fx.agents:
            clone = os.path.join(scratch, "clones", ag["name"])
            git(["clone", "-q", origin, clone], scratch, henv)
            private = fx.private_files(ag["name"])
            with open(os.path.join(clone, ".git", "info", "exclude"), "a") as f:
                f.write("\n.claude/\n" + "".join("/%s\n" % rel for rel, _ in private))
            for rel, src in private:  # this agent's private knowledge: in its clone only, never committed
                os.makedirs(os.path.dirname(os.path.join(clone, rel)), exist_ok=True)
                shutil.copyfile(src, os.path.join(clone, rel))
            clones[ag["name"]] = clone
        kenv = {"KLATCH_HOME": khome, "KLATCH_KEY_DIR": kkeys, "KLATCH_BIN": a.klatch_bin or "", "KLATCH_RUN_DIR": scratch}
        if arm.startswith("K") and arm != "K1":
            for name, clone in clones.items():
                e = dict(henv, **kenv, KLATCH_NAME=name)
                p = subprocess.run([a.klatch_bin, "init", "--scope", "project", "--yes", "--as-member", name],
                                   cwd=clone, env=e, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
                if p.returncode != 0:
                    raise RuntimeError("klatch init failed for %s: %s" % (name, p.stdout.strip()[-200:]))
        if arm.startswith("K") and (fx.seed_history or {}).get("room"):
            self.seed_room(fx.seed_history["room"], scratch, dict(henv, **kenv), notes)
        t0 = time.monotonic()
        first_pass = None
        last_poll = t0
        outcome = None
        for ag in fx.agents:
            name = ag["name"]
            prompt = build_prompt(a.arms_dir, arm, name, team, ag["task"], clones[name], fx.test_command,
                                  a.klatch_bin, a.room, a.max_turns, max(1, int(a.wall_s // 60)))
            with open(os.path.join(log_dir, "%s.prompt.md" % name), "w") as f:
                f.write(prompt)
            argv, extra = self.driver.command(name, arm, prompt, clones[name], log_dir)
            env = self.driver.agent_env(os.environ, scratch)
            env.update(henv)  # HOME is the run's own
            # each agent commits under its own name, so git authorship says who did what (duplicate work, per-agent
            # contribution); the setup commits keep the harness name
            env.update(GIT_AUTHOR_NAME=name, GIT_AUTHOR_EMAIL=name + "@bench.invalid", GIT_COMMITTER_NAME=name,
                       GIT_COMMITTER_EMAIL=name + "@bench.invalid")
            env.update(kenv)
            env.update(extra)
            env["KLATCH_NAME"] = name
            env["TMPDIR"] = scratch
            logf = open(os.path.join(log_dir, "%s.stream.jsonl" % name), "wb")
            errf = open(os.path.join(log_dir, "%s.err" % name), "wb")
            if a.stagger_s:
                time.sleep(rng.uniform(0, a.stagger_s))
            procs[name] = subprocess.Popen(argv, cwd=clones[name], env=env, stdin=subprocess.DEVNULL, stdout=logf,
                                           stderr=errf, start_new_session=True)
            logf.close()
            errf.close()
            states[name] = streams.StreamState(klatch_bin=a.klatch_bin)
            offs[name], partial[name] = 0, b""
        t0 = time.monotonic()

        def pump():
            now = round(time.monotonic() - t0, 3)
            for name in procs:
                with open(os.path.join(log_dir, "%s.stream.jsonl" % name), "rb") as f:
                    f.seek(offs[name])
                    chunk = f.read()
                offs[name] += len(chunk)
                data = partial[name] + chunk
                *lines, partial[name] = data.split(b"\n")
                for ln in lines:
                    states[name].feed(ln.decode("utf-8", "replace"), now)

        def checks_now(tag):
            dest = os.path.join(scratch, "check-" + tag)
            shutil.rmtree(dest, ignore_errors=True)
            git(["clone", "-q", "-b", "main", origin, dest], scratch, henv)
            return fx.run_checker(dest, henv)

        def all_pass(checks):
            total, done = schema.count_units(checks)
            return total > 0 and total == done

        try:
            while True:
                time.sleep(0.05)
                pump()
                elapsed = time.monotonic() - t0
                if sum(s.cost_usd for s in states.values()) >= a.max_run_usd:
                    outcome = "spend-cap"
                    break
                if elapsed >= a.wall_s:
                    outcome = "timeout"
                    break
                if all(p.poll() is not None for p in procs.values()):
                    break
                if first_pass is None and time.monotonic() - last_poll >= a.poll_s:
                    last_poll = time.monotonic()
                    checks, _, err = checks_now("poll")
                    if err is None and all_pass(checks):
                        first_pass = time.monotonic() - t0
        finally:
            for p in procs.values():
                kill_group(p)
            for p in procs.values():
                p.wait()
        pump()
        wall = time.monotonic() - t0
        rcs = {n: p.returncode for n, p in procs.items()}
        if outcome is None:
            outcome = "crash" if any(rc != 0 for rc in rcs.values()) else "ok"
        if outcome == "crash":
            notes.append("agent exit codes %s" % json.dumps(rcs, sort_keys=True))
        # the final hidden check on a clone of the final origin
        checks, raw, err = checks_now("final")
        with open(os.path.join(log_dir, "check.json"), "w") as f:
            f.write(raw)
        if err:
            notes.append("checker error: " + err)
        if first_pass is None and err is None and all_pass(checks):
            first_pass = wall
        sums = {n: s.summary() for n, s in states.items()}
        total, done = schema.count_units(checks)
        shas = {n: sha256_file(os.path.join(log_dir, "%s.stream.jsonl" % n)) for n in procs}
        any_est = any(s["cost_estimated"] for s in sums.values())
        models = []
        for s in sums.values():
            models += [m for m in s["models"] if m not in models]
        rec["models"]["resolved_ids"] = models
        rec["wall_s"] = round(wall, 3)
        rec["time_to_first_pass_s"] = None if first_pass is None else round(first_pass, 3)
        rec["outcome"] = outcome
        rec["spend"] = {"usd": round(sum(s["cost_usd"] for s in sums.values()), 6), "estimated": any_est}
        rec["turns"] = {"total": sum(s["turns"] for s in sums.values()), "per_agent": {n: s["turns"] for n, s in sums.items()}}
        rec["tokens"] = {k: sum(s["tokens"][k] for s in sums.values()) for k in ("input", "output", "cache_read", "cache_creation")}
        by_tool = {}
        for s in sums.values():
            for k, v in s["tool_calls"]["by_tool"].items():
                by_tool[k] = by_tool.get(k, 0) + v
        rec["tool_calls"] = {"total": sum(by_tool.values()), "by_tool": by_tool}
        coord_chars = sum(s["coord_context_chars"] for s in sums.values())
        rec["metrics"].update(
            units_total=total, units_completed=done,
            git_conflicts=sum(s["git_conflicts"] for s in sums.values()),
            messages=sum(s["messages"] for s in sums.values()),
            human_interventions=sum(s["human_mentions"] for s in sums.values()),
            coordination_context_tokens=coord_chars // 4,
            idle_turns=sum(s["idle_turns"] for s in sums.values()))
        notes.append("coordination_context_tokens estimated as characters/4 (no tokenizer in the standard library)")
        if any(s["is_error"] for s in sums.values()):
            notes.append("an agent's result message reported is_error")
        rec["outcome_checks"] = checks
        self.scenario_metrics(rec, raw, origin, list(clones.values()), henv, notes, states)
        rec["artifacts"] = {"streams": log_dir, "store_export": None,
                            "checker_report": os.path.join(log_dir, "check.json"), "stream_sha256": shas}

    def seed_room(self, messages, scratch, env, notes):
        """Post the fixture's seeded earlier exchange into the room, each message as its own member, before any agent starts."""
        joined = set()
        for m in messages:
            e = dict(env, KLATCH_NAME=m["as"])
            for argv in ([["join", self.args.room]] if m["as"] not in joined else []) + [["post", self.args.room, m["text"]]]:
                p = subprocess.run([self.args.klatch_bin] + argv, cwd=scratch, env=e, stdout=subprocess.PIPE,
                                   stderr=subprocess.STDOUT, text=True)
                if p.returncode != 0:
                    raise RuntimeError("seeding the room failed (%s): %s" % (" ".join(argv[:1]), p.stdout.strip()[-200:]))
            joined.add(m["as"])

    def scenario_metrics(self, rec, raw, origin, clone_dirs, henv, notes, states=None):
        """Fill duplicate_work and overwrites from git history for a fixture that has a scenario file, and keep the
        checker's extra fields under scenario_metrics. A fixture without a scenario leaves them null."""
        name = self.fx.spec.get("scenario")
        if not name:
            return
        try:
            scn = scenario_mod.load(name, self.args.scenarios_dir)
            gm = scenario_mod.git_metrics(scn, origin, clone_dirs, henv)
        except (scenario_mod.ScenarioError, RuntimeError, KeyError, OSError) as e:
            notes.append("scenario metrics failed: %s" % e)
            return
        rec["metrics"]["duplicate_work"] = gm["duplicate_work"]
        rec["metrics"]["overwrites"] = gm["overwrites"]
        rec["scenario_metrics"] = dict(gm["details"], **scenario_mod.checker_extras(raw))
        if scn.get("slots") and states is not None:
            self.ask_metrics(rec, scn, states, notes)

    def ask_metrics(self, rec, scn, states, notes):
        """The S4 ask and answer metrics from the agents' message events (asks.py); the checker's slot states give the
        guesses and unanswered dependencies. Skipped, with a note, when the checker's slots are not there."""
        slots = rec["scenario_metrics"].get("slots")
        if not isinstance(slots, dict):
            notes.append("ask metrics skipped: the checker printed no slots")
            return
        try:
            with open(os.path.join(self.checks_dir_for(), self.fx.spec["checker"], "facts.json")) as f:
                facts = json.load(f)["facts"]
        except (OSError, ValueError, KeyError) as e:
            notes.append("ask metrics skipped: no facts for the checker (%s)" % e)
            return
        events = sorted(([dict(e, agent=n) for n, s in states.items() for e in s.msg_events]),
                        key=lambda e: (e["t"] if e["t"] is not None else 0))
        done = {n: s.done_t for n, s in states.items() if s.done_t is not None}
        rec["scenario_metrics"]["asks"] = asks_mod.compute(events, done, scn, facts, slots)

    def checks_dir_for(self):
        return self.fx.checks_dir

    # the suite ---------------------------------------------------------------------------------------------

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
        probe_home = tempfile.mkdtemp(prefix="cb-probe-", dir=self.tmp_root)
        try:
            self.klatch_facts = klatch_facts(a.klatch_bin, minimal_env(os.environ, probe_home))
        finally:
            shutil.rmtree(probe_home, ignore_errors=True)
        self.commit = a.klatch_commit or commit_of(HERE)
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
            sys.stdout.flush()
        self.write_meta(started, now_iso(), not_run)
        print("done: %d of %d runs, %.4f USD, results in %s" % (self.executed, len(self.plan), self.spent, self.results_root))
        return 3 if self.cap_hit else (1 if self.failed else 0)


def commit_of(path):
    try:
        p = subprocess.run(["git", "-C", path, "rev-parse", "HEAD"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                           text=True, timeout=30)
        return p.stdout.strip() if p.returncode == 0 and p.stdout.strip() else None
    except (OSError, subprocess.SubprocessError):
        return None


def parse_args(argv):
    p = argparse.ArgumentParser(description="Run a coordination bench scenario against arms, N times each (#1364).")
    p.add_argument("--fixture", required=True)
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
    p.add_argument("--room", default="lobby")
    p.add_argument("--seed", type=int, default=1000)
    p.add_argument("--model", help="the agent model id (required with --driver claude)")
    p.add_argument("--roles-model")
    p.add_argument("--claude-bin", default="claude")
    p.add_argument("--fake-script", help="a fake-agent script (default: the fixture's)")
    p.add_argument("--max-run-usd", type=float, default=None, help="per-run spend cap (default %g; with an approval file "
                   "the file's cap)" % DEFAULT_RUN_USD)
    p.add_argument("--max-suite-usd", type=float, default=None, help="suite spend cap (default %g; with an approval file "
                   "the file's budget)" % DEFAULT_SUITE_USD)
    p.add_argument("--wall-s", type=float, default=1800)
    p.add_argument("--max-turns", type=int, default=60, help="turn cap per agent")
    p.add_argument("--poll-s", type=float, default=60.0, help="how often the hidden checker is polled while agents run")
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


def main(argv=None):
    a = parse_args(argv if argv is not None else sys.argv[1:])
    a.arm_list = [x for x in a.arm.split(",") if x]
    if a.runs < 1 or not a.arm_list:
        print("run.py: need at least one arm and one run", file=sys.stderr)
        return 2
    for arm in a.arm_list:
        if not os.path.exists(os.path.join(a.arms_dir, arm + ".md")):
            print("run.py: no arm prompt %s.md in %s" % (arm, a.arms_dir), file=sys.stderr)
            return 2
    if any(x.startswith("K") for x in a.arm_list) and not a.klatch_bin and not a.dry_run:
        print("run.py: the K arms need --klatch-bin (the absolute path of the klatch binary)", file=sys.stderr)
        return 2
    if a.klatch_bin and not os.path.isabs(a.klatch_bin):
        a.klatch_bin = os.path.abspath(a.klatch_bin)
    try:
        fx = fixture_mod.Fixture(a.fixture, a.fixtures_dir, a.checks_dir)
    except (fixture_mod.FixtureError, KeyError) as e:
        print("run.py: %s" % e, file=sys.stderr)
        return 2
    plan = plan_runs(a.arm_list, a.runs, a.seed)
    if a.dry_run:
        return dry_run(a, fx, plan)
    if a.driver == "claude":
        if not a.budget_approved_in:
            print("run.py: real agents cost money; --budget-approved-in must name the budget approval file the human "
                  "made (SPEC section 8, approval.py). Without it only --driver fake runs.", file=sys.stderr)
            return 2
        if not a.model:
            print("run.py: --driver claude needs --model (a pinned model id)", file=sys.stderr)
            return 2
        try:
            a.max_run_usd, a.max_suite_usd = resolve_caps(a)
        except approval_mod.ApprovalError as e:
            print("run.py: %s" % e, file=sys.stderr)
            return 2
        driver = drivers.ClaudeDriver(a.claude_bin, a.model, a.max_turns, a.max_run_usd / len(fx.agents), a.klatch_bin)
    else:
        script = a.fake_script or fx.fake_script
        if not script:
            print("run.py: the fixture has no fake script; give --fake-script", file=sys.stderr)
            return 2
        driver = drivers.FakeDriver(os.path.abspath(script), a.klatch_bin, a.room)
    a.tmp_root = os.path.realpath(a.tmp_root)
    try:
        guard_paths(os.path.join(a.tmp_root, "x"), os.path.join(a.tmp_root, "y"), a.tmp_root)
    except GuardError as e:
        print("run.py: %s" % e, file=sys.stderr)
        return 2
    return Suite(a, fx, driver, plan).go()


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


if __name__ == "__main__":
    sys.exit(main())
