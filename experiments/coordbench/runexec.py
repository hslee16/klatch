"""One run of the harness, from the scratch directory to the filled record (split out of run.py, #1455).

RunExecution is what Suite.execute used to be: a function with a closure of local state. The state is now attributes and the
phases are methods, in the order they ran: directories and guard, the bare origin, the scenario, the roles, the clones, the
klatch members, the room, the agent processes, the watch loop, the end of the run, the final check and the record's numbers.
The numbers beyond the streams are in runmetrics.RunMetrics (a mixin). git is passed in by run.py, so that a test that
patches run.git still reaches every git call of a run."""
import collections
import json
import os
import random
import shlex
import shutil
import subprocess
import time

import agentcap
import history as history_mod
import injections
import recovery
import roles as roles_mod
import scenario as scenario_mod
import schema
import shared as shared_mod
import streams
from runenv import EXCLUDE, append_exclude, guard_paths, kill_group, minimal_env, sha256_file
from runmetrics import RunMetrics, resolved_models, team_metrics, tool_calls_of
from runisolation import IsolationSupport
from runpoll import PollSupport
from runroles import RoleSupport
from runprompt import TASK_BOARD, build_prompt, seed_board_attributes

HERE = os.path.dirname(os.path.abspath(__file__))
RunSpot = collections.namedtuple("RunSpot", "arm seed scratch log_dir")  # which arm, with which seed, where it runs and logs
class RunExecution(RunMetrics, RoleSupport, IsolationSupport, PollSupport):
    def __init__(self, suite, git, rec, spot, notes):
        self.git = git
        self.a, self.fx, self.driver = suite.args, suite.fx, suite.driver
        self.tmp_root = suite.tmp_root
        self.rec, self.notes = rec, notes
        self.arm, self.seed, self.scratch, self.log_dir = spot
        self.is_shared = self.fx.topology == "T-shared"
        self.shared_tree = os.path.join(self.scratch, "shared")
        self.minutes = max(1, int(self.a.wall_s // 60))
        self.procs, self.states, self.offs, self.partial = {}, {}, {}, {}
        self.stamps = {}  # session key -> seconds since the agents started, one per stream line (timeline.json)
        self.agent_of = {}  # session key -> agent name; a restarted agent has a second session, key "<name>.r2"
        self.role_keys, self.repl_keys = set(), set()  # the role members, and the replacements the supervisor started (#1421)
        self.started_at, self.ended_at, self.stop_reason, self.restarts = {}, {}, {}, []
        self.clones = {}
        self.kill_rec = self.killer = self.victim_name = None
        self.snaps = self.watcher = self.shared_info = None
        self.pending, self.cut = {}, set()
        self.first_pass = self.outcome = self.grace_start = None
        self.agent_caps = {}
        self.inj = self.inj_plan_path = None
        self.rcfg, self.role_names, self.role_info = None, [], {}
        self.restart_dir, self.restart_command = None, ""
        self.metric_states = {}
        self.init_isolation()

    def execute(self):
        self.prepare_directories()
        self.build_origin()
        self.load_scenario()
        self.configure_roles()
        self.make_clones()
        self.init_klatch_members()
        self.prepare_room()
        self.start_agents()
        self.watch()
        self.settle()
        self.final_check()
        self.fill_record()

    # before the agents start ---------------------------------------------------------------------------------

    def prepare_directories(self):
        home = os.path.join(self.scratch, "home")
        self.khome = os.path.join(self.scratch, "klatch-home")
        self.kkeys = os.path.join(self.scratch, "klatch-keys")
        for d in (home, self.khome, self.kkeys):
            os.makedirs(d)
            os.chmod(d, 0o700)
        guard_paths(self.khome, self.kkeys, self.tmp_root)
        self.henv = minimal_env(os.environ, home)
        # the repository: initial tree from the fixture, a bare origin made for this run
        self.seedtree = os.path.join(self.scratch, "seed")
        self.fx.materialise(self.seedtree, self.seed, self.henv, arm=self.arm)

    def plan_injections(self):
        """S10 (#1375): the scripted colleague's injections for this run. The plan (tokens, paths, texts) goes to the log
        directory for the hidden checker and the fake agents; a real agent is never given the file. The harness makes the
        directory a "touch" injection names, so an obedient agent's touch can succeed and the check finds the file."""
        fx = self.fx
        self.inj = injections.plan(self.seed, self.scratch, self.arm, self.a.room, fx.injection_mode, fx.injection)
        self.inj_plan_path = os.path.join(self.log_dir, "injection-plan.json")
        with open(self.inj_plan_path, "w") as f:
            json.dump(self.inj, f, indent=1)
        os.makedirs(os.path.join(self.scratch, "pwned"), mode=0o700, exist_ok=True)

    def build_origin(self):
        fx, git, henv, scratch = self.fx, self.git, self.henv, self.scratch
        if fx.injection:
            self.plan_injections()
        self.origin = os.path.join(scratch, "origin.git")
        git(["init", "-q", "--bare", "-b", "main", self.origin], scratch, henv)
        if fx.prebuilt_history:
            # setup.sh built a repository with a scripted history (it knows the arm, and in B1 put the board files into it)
            git(["push", "-q", self.origin, "HEAD:main"], self.seedtree, henv)
        else:
            self.commit_initial_tree()
        shutil.rmtree(self.seedtree)

    def commit_initial_tree(self):
        git, henv, seedtree = self.git, self.henv, self.seedtree
        if self.arm in ("B1", "B1-roles"):
            board = (self.fx.seed_history or {}).get("board", []) + (self.inj["board"] if self.inj else [])
            with open(os.path.join(seedtree, "TASKS.md"), "w") as f:
                f.write(TASK_BOARD + "".join(l + "\n" for l in board))
            seed_board_attributes(seedtree)
        git(["init", "-q", "-b", "main"], seedtree, henv)
        git(["add", "-A"], seedtree, henv)
        git(["commit", "-q", "-m", "initial"], seedtree, henv)
        git(["push", "-q", self.origin, "HEAD:main"], seedtree, henv)

    def load_scenario(self):
        self.scn = None
        if self.fx.spec.get("scenario"):
            try:
                self.scn = scenario_mod.load(self.fx.spec["scenario"], self.a.scenarios_dir)
            except scenario_mod.ScenarioError:
                self.scn = None
        self.kill_spec = recovery.kill_spec(self.scn, self.fx.variant) if self.scn else None
        self.rng = random.Random(self.seed)
        self.team = self.fx.names

    def configure_roles(self):
        if not roles_mod.is_role_arm(self.arm):
            return
        try:
            self.rcfg = roles_mod.resolve(self.a, self.scn, self.arm, len(self.team))
        except ValueError as e:
            raise RuntimeError(str(e))
        clash = [m for m in self.rcfg["members"] if m in self.team]
        if clash:
            raise RuntimeError("a worker is named like a role member: %s (the role members are named %s)"
                               % (", ".join(clash), ", ".join(roles_mod.ROLES)))
        self.role_names = self.rcfg["members"]

    def klatch_init(self, name, clone):
        e = dict(self.henv, **self.kenv, KLATCH_NAME=name)
        p = subprocess.run([self.a.klatch_bin, "init", "--scope", "project", "--yes", "--as-member", name],
                           cwd=clone, env=e, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        if p.returncode != 0:
            raise RuntimeError("klatch init failed for %s: %s" % (name, p.stdout.strip()[-200:]))

    def init_klatch_members(self):
        self.kenv = {"KLATCH_HOME": self.khome, "KLATCH_KEY_DIR": self.kkeys, "KLATCH_BIN": self.a.klatch_bin or "",
                     "KLATCH_RUN_DIR": self.scratch}
        if self.arm.startswith("K") and self.arm != "K1":
            # one init per working directory: in a shared tree the hooks it writes are the same for everyone, and each
            # agent's process names itself with KLATCH_NAME (set below)
            for name, clone in (list(self.clones.items())[:1] if self.is_shared else self.clones.items()):
                self.klatch_init(name, clone)

    def prepare_restart_dir(self):
        self.restart_dir = os.path.join(self.scratch, "restart")
        os.makedirs(self.restart_dir, mode=0o700)
        with open(os.path.join(self.restart_dir, "workers"), "w") as f:
            f.write("".join(n + "\n" for n in self.team))
        # the script is copied into the run: no prompt names a path inside the repository (#1519)
        os.makedirs(os.path.join(self.scratch, "bin"), exist_ok=True)
        script = shutil.copy(os.path.join(HERE, "restart_member.py"), os.path.join(self.scratch, "bin", "restart_member.py"))
        self.restart_command = "python3 %s" % shlex.quote(script)
        self.check_role_launch()

    def post_before_agents(self, env):
        """K arms: the fixture's seeded exchange, then the scripted colleague's injections, through the binary."""
        a, fx, inj = self.a, self.fx, self.inj
        if (fx.seed_history or {}).get("room"):
            self.seed_room(fx.seed_history["room"], env)
        if inj and inj["room_steps"]:
            # the scripted colleague posts, claims, asks and sends a dm through the binary, before the agent starts
            history_mod.post_room(a.klatch_bin, a.room, inj["room_steps"], env, self.scratch)

    def prepare_room(self):
        a, fx = self.a, self.fx
        is_k = self.arm.startswith("K")
        kenv_run = dict(self.henv, **self.kenv)
        self.role_info = {r: {"granted": None} for r in self.role_names}
        if self.rcfg and is_k:
            self.grant_roles(kenv_run)
        if self.rcfg:
            self.prepare_restart_dir()
        if is_k:
            self.post_before_agents(kenv_run)
        self.t0 = time.monotonic()
        self.last_poll = self.t0
        if is_k and fx.room_script:
            # the earlier members' room history, posted through the binary into this run's temporary store
            history_mod.post_room(a.klatch_bin, a.room, fx.room_script, kenv_run, self.scratch)

    # the agent processes -------------------------------------------------------------------------------------

    def prompt_and_kw(self, name, task, role, replacement):
        """The prompt of a session and the driver keywords: a role member has the role's prompt, model and budget; a
        replacement the supervisor asked for is a worker charged to the roles."""
        a, fx, rcfg = self.a, self.fx, self.rcfg
        if role:
            prompt = roles_mod.role_prompt(a.arms_dir, self.arm, role, name, self.team, self.clones[name], fx.test_command,
                                           a.klatch_bin, a.room, self.restart_command, a.max_turns, self.minutes)
            return prompt, {"role": role, "model": rcfg["model"], "max_usd": rcfg["per_role_usd"]}
        prompt = build_prompt(a.arms_dir, self.arm, name, self.team, task, self.clones[name], fx.test_command,
                              a.klatch_bin, a.room, a.max_turns, self.minutes, topology=fx.topology)
        kw = {}
        if rcfg:  # the run cap is spread over every member, the roles included
            kw = {"max_usd": rcfg["per_role_usd"] if replacement else a.max_run_usd / float(len(self.team) + len(self.role_names))}
        return prompt, kw

    def agent_env(self, name, extra, role):
        agent_dir = self.agent_dir(name)  # HOME, TMPDIR and the config directory are the agent's own, never the run's
        env = self.driver.agent_env(os.environ, agent_dir)
        env.update(self.henv)  # HOME is the run's own
        # each agent commits under its own name, so git authorship says who did what (duplicate work, per-agent
        # contribution); the setup commits keep the harness name
        env.update(GIT_AUTHOR_NAME=name, GIT_AUTHOR_EMAIL=name + "@bench.invalid", GIT_COMMITTER_NAME=name,
                   GIT_COMMITTER_EMAIL=name + "@bench.invalid")
        env.update(self.kenv)
        env.update(extra)
        env["KLATCH_NAME"] = name
        env["HOME"] = os.path.join(agent_dir, "home")
        if "CLAUDE_CONFIG_DIR" in env:  # the agent's own, or in B1 and B1-roles the run's one shared directory (#1520)
            env["CLAUDE_CONFIG_DIR"] = self.config_dir(name)
        env["TMPDIR"] = os.path.join(agent_dir, "tmp")
        if role:
            env["COORDBENCH_RESTART_DIR"] = self.restart_dir
        return env

    def start(self, key, name, session_no, task, extra=None):
        """Start one session of an agent: its prompt, its process, its stream log and parser. extra: max_turns (a session
        that is cut), role (a role member) or replacement (a worker the supervisor asked for)."""
        a, log_dir = self.a, self.log_dir
        extra = extra or {}
        role, replacement = extra.get("role"), extra.get("replacement", False)
        prompt, kw = self.prompt_and_kw(name, task, role, replacement)
        for kind in ("prompt", "stream", "err"):  # a real agent's log files have random names until the run ends (#1519)
            self.log_path(key, kind)
        with open(self.log_path(key, "prompt"), "w") as f:
            f.write(prompt)
        argv, driver_extra = self.driver.command(name, self.arm, prompt, self.clones[name], log_dir, session=session_no,
                                          max_turns=extra.get("max_turns"), isolation=self.iso_for(name, bool(role)), **kw)
        env = self.agent_env(name, driver_extra, role)
        logf = open(self.log_path(key, "stream"), "wb")
        errf = open(self.log_path(key, "err"), "wb")
        if a.stagger_s and session_no == 1:
            time.sleep(self.rng.uniform(0, a.stagger_s))
        self.procs[key] = subprocess.Popen(argv, cwd=self.clones[name], env=env, stdin=subprocess.DEVNULL, stdout=logf,
                                           stderr=errf, start_new_session=True)
        logf.close()
        errf.close()
        self.states[key] = streams.StreamState(klatch_bin=a.klatch_bin)
        self.offs[key], self.partial[key] = 0, b""
        self.stamps[key] = []
        self.agent_of[key] = name
        self.started_at[key] = time.monotonic()
        if role:
            self.role_keys.add(key)
        if replacement:
            self.repl_keys.add(key)

    def start_agents(self):
        fx = self.fx
        self.task_of = {ag["name"]: ag["task"] for ag in fx.agents}
        self.hide_run()  # from here nobody can list the scratch and log directories until the agents are stopped
        # a resume variant (fixture "resume"): the agent's first session is cut after after_turns turns and, once it has
        # exited, a second session with the same name starts in the same clone with the job text and a short note that its
        # context was cleared. Nothing else of the first session is carried over.
        resume = fx.resume
        for ag in fx.agents:
            name = ag["name"]
            is_cut = bool(resume) and resume["agent"] == name
            self.start(name, name, 1, ag["task"], {"max_turns": resume["after_turns"]} if is_cut else None)
            if is_cut:
                self.cut.add(name)
                self.pending[name + ".r2"] = (name, name, ag["task"] + "\n\n" + resume["note"])
        for r in self.role_names:  # the role members start beside the workers, each its own process
            self.start(r, r, 1, None, {"role": r})
        self.t0 = time.monotonic()
        self.start_watchers()

    def wants_snapshots(self):
        return self.is_shared and bool(self.scn) and bool(self.scn.get("shared_check"))

    def start_watchers(self):
        scn = self.scn
        if self.wants_snapshots():
            # light watcher thread: the shared file as it is on disk, every --snapshot-s seconds and when an agent exits
            self.snaps = shared_mod.Snapshots()
            self.watcher = shared_mod.Watcher(os.path.join(self.shared_tree, scn["shared_check"]["path"]), self.a.snapshot_s,
                                              lambda: dict(self.procs), self.t0, self.snaps)
            self.watcher.start()
        if self.kill_spec:
            # scenario S6: the victim is killed when the scenario's conditions hold on its stream and the git history
            self.victim_name = self.kill_spec["agent"]
            unit_paths = [scn["units"][u]["path"] for u in scn["agents"][self.victim_name]["units"]]
            self.killer = recovery.Killer(self.kill_spec, self.arm, unit_paths, recovery.GitProbe(self.origin, self.henv))

    # watching them -------------------------------------------------------------------------------------------

    def checks_now(self, tag):
        dest = os.path.join(self.scratch, "check-" + tag)
        shutil.rmtree(dest, ignore_errors=True)
        if self.is_shared and tag == "poll":
            # the tree as it is on disk now, uncommitted work included (agents are still running)
            try:
                shutil.copytree(self.shared_tree, dest, symlinks=True)
            except (shutil.Error, OSError) as e:
                return [], "", "could not copy the shared tree: %s" % e
        elif self.is_shared:
            self.git(["clone", "-q", "--no-local", self.shared_tree, dest], self.scratch, self.henv)  # HEAD of the tree, after the final commit
        else:
            # --no-local: a clone by path walks and links objects/ file by file, and while an agent's push is in flight
            # that directory holds quarantine files that vanish mid-walk (#1435); upload-pack reads a consistent snapshot
            self.git(["clone", "-q", "--no-local", "-b", "main", self.origin, dest], self.scratch, self.henv)
        env = dict(self.henv, COORDBENCH_INJECTION_PLAN=self.inj_plan_path) if self.inj_plan_path else self.henv
        return self.fx.run_checker(dest, env)

    @staticmethod
    def all_pass(checks):
        total, done = schema.count_units(checks)
        return total > 0 and total == done

    def maybe_kill(self, elapsed):
        """Evaluate the kill trigger; SIGKILL the victim's process group when it is due (no hook runs, nothing is
        released) and write kill.json, which a scripted fake survivor may wait for. Returns the record or None."""
        proc = self.procs[self.victim_name]
        if proc.poll() is not None:
            return {"fired": False, "agent": self.victim_name,
                    "reason": "the victim exited (code %s) before the trigger fired" % proc.returncode}
        with open(self.log_path(self.victim_name, "stream"), errors="replace") as f:
            lines = f.read().splitlines()
        due = self.killer.poll(lines)
        if due is None:
            return None
        kill_group(proc)
        proc.wait()
        self.pump()
        rec_ = {"fired": True, "agent": self.victim_name, "trigger": due["trigger"], "turns": due["turns"],
                "conditions": due["conditions"], "observed": due.get("observed"), "t": round(elapsed, 3), "epoch": time.time(),
                "lines": {k: len(v) for k, v in self.stamps.items()}}
        tmp = os.path.join(self.log_dir, "kill.json.tmp")
        with open(tmp, "w") as f:
            json.dump(rec_, f, sort_keys=True)
        os.replace(tmp, os.path.join(self.log_dir, "kill.json"))
        return rec_

    def cap_outcome(self, elapsed):
        """spend-cap or timeout when the run hit one (the roles' spend counts), else None."""
        if sum(s.cost_usd for s in self.states.values()) >= self.a.max_run_usd:
            return "spend-cap"
        return "timeout" if elapsed >= self.a.wall_s else None

    def start_pending(self):
        for key, (after, name, task) in list(self.pending.items()):
            if self.procs[after].poll() is not None:
                self.pump()
                del self.pending[key]
                self.start(key, name, 2, task)

    def note_exits(self):
        for k, p in self.procs.items():
            if p.poll() is not None and k not in self.ended_at:
                self.ended_at[k] = time.monotonic()

    def workers_done_and_roles_stopped(self):
        """True when the loop should end: every worker has ended and the roles have too, or their grace is over (the
        integrator's last merge), when they are stopped."""
        workers_ended = not self.pending and all(p.poll() is not None for k, p in self.procs.items() if k not in self.role_keys)
        if not workers_ended:
            self.grace_start = None
            return False
        live = [k for k in self.role_keys if self.procs[k].poll() is None]
        return not live or self.grace_is_over(live)

    def grace_is_over(self, live):
        """The roles still running get rcfg grace_s seconds after the last worker ended; then they are stopped."""
        if self.grace_start is None:
            self.grace_start = time.monotonic()
        if time.monotonic() - self.grace_start < self.rcfg["grace_s"]:
            return False
        for k in live:
            self.stop_reason[k] = "stopped-by-harness"
        return True

    def tick(self):
        """One turn of the watch loop; True when the loop should end."""
        time.sleep(0.05)
        self.pump()
        elapsed = time.monotonic() - self.t0
        if self.killer and self.kill_rec is None:
            self.kill_rec = self.maybe_kill(elapsed)
        self.outcome = self.cap_outcome(elapsed)
        if self.outcome:
            return True
        self.start_pending()
        if self.rcfg:
            self.handle_restarts()
        self.note_exits()
        if self.workers_done_and_roles_stopped():
            return True
        self.poll_checker()
        return False

    def watch(self):
        try:
            while not self.tick():
                pass
        finally:
            for k, p in self.procs.items():
                if p.poll() is None and k in self.role_keys:
                    self.stop_reason.setdefault(k, "killed-by-cap")
            for p in self.procs.values():
                kill_group(p)
            for p in self.procs.values():
                p.wait()
            self.show_run()
            self.join_poll()

    # the end of the run --------------------------------------------------------------------------------------

    def settle_kill(self):
        """The kill record of scenario S6 (a default one when the trigger never fired) and the timeline file."""
        if not self.killer:
            return
        if self.kill_rec is None:
            self.kill_rec = {"fired": False, "agent": self.victim_name, "reason": "the run ended before the trigger fired"}
        if not self.kill_rec["fired"]:
            self.outcome = recovery.outcome_when_unfired(self.kill_rec, self.outcome)
            self.notes.append("the kill never fired: %s" % self.kill_rec["reason"])
        with open(os.path.join(self.log_dir, "timeline.json"), "w") as f:
            json.dump(self.stamps, f, sort_keys=True)

    def finish_shared_tree(self):
        """T-shared: stop the watcher and commit the tree; the tree, committed, is the final state. None otherwise."""
        if not self.is_shared:
            return None
        if self.watcher:
            self.watcher.finish()
        return shared_mod.finalise(self.shared_tree, self.henv)

    def crashed(self, killed):
        """Some worker exited nonzero (a cut session, the killed victim and the role members do not count)."""
        return any(rc != 0 for k, rc in self.rcs.items() if k not in self.cut and k != killed and k not in self.role_keys)

    def capped_agents(self, killed):
        """{outcome: [session keys]} of the workers that stopped on their own spend or turn cap (#1553)."""
        return agentcap.agent_caps({k: s.summary() for k, s in self.states.items()
                                    if k not in self.cut and k != killed and k not in self.role_keys})

    def settle(self):
        self.stopped_t = time.monotonic()
        self.pump()
        self.publish_log_names()
        self.final_commit = self.finish_shared_tree()
        self.wall = time.monotonic() - self.t0
        self.rcs = {n: p.returncode for n, p in self.procs.items()}
        killed = self.victim_name if self.kill_rec and self.kill_rec.get("fired") else None
        self.settle_kill()
        if self.outcome is None:
            self.agent_caps = self.capped_agents(killed)
            self.outcome = agentcap.cap_outcome(self.agent_caps) or ("crash" if self.crashed(killed) else "ok")
        if self.outcome == "crash":
            self.notes.append("agent exit codes %s" % json.dumps(self.rcs, sort_keys=True))

    def passes_cleanly(self, err):
        return err is None and self.all_pass(self.checks)

    def final_check(self):
        """The final hidden check on a clone of the final origin."""
        self.checks, self.raw, err = self.checks_now("final")
        with open(os.path.join(self.log_dir, "check.json"), "w") as f:
            f.write(self.raw)
        if err:
            self.notes.append("checker error: " + err)
        if self.first_pass is None and self.passes_cleanly(err):
            self.first_pass = self.wall

    # the record's numbers ------------------------------------------------------------------------------------

    def fill_totals(self, sums, allsums):
        """The record's spend, turns, tokens, tool calls and metrics from the stream summaries. sums are the workers'
        (the team's work); allsums add the role members' and the supervisor's replacements', which are the roles' (#1421)."""
        rec = self.rec
        total, done = schema.count_units(self.checks)
        rec["models"]["resolved_ids"] = resolved_models(allsums)
        rec["wall_s"] = round(self.wall, 3)
        rec["time_to_first_pass_s"] = None if self.first_pass is None else round(self.first_pass, 3)
        rec["outcome"] = self.outcome
        rec["spend"] = {"usd": round(sum(s["cost_usd"] for s in allsums.values()), 6),
                        "estimated": any(s["cost_estimated"] for s in allsums.values())}
        rec["turns"] = {"total": sum(s["turns"] for s in sums.values()), "per_agent": self.turns_per_agent(sums)}
        rec["tokens"] = {k: sum(s["tokens"][k] for s in sums.values()) for k in ("input", "output", "cache_read", "cache_creation")}
        rec["tool_calls"] = tool_calls_of(sums)
        rec["metrics"].update(units_total=total, units_completed=done, **team_metrics(sums))

    def turns_per_agent(self, sums):
        per_agent = {}
        for k, s in sums.items():
            per_agent[self.agent_of[k]] = per_agent.get(self.agent_of[k], 0) + s["turns"]
        return per_agent

    def note_roles_and_errors(self, sums):
        if self.rcfg:
            self.role_record()
        self.notes.extend(agentcap.notes(self.agent_caps, self.allsums, self.agent_of))
        if any(s["is_error"] for s in sums.values()):
            self.notes.append("an agent's result message reported is_error")

    def scenario_numbers(self, sums):
        if self.is_shared:
            self.shared_info = {"tree": self.shared_tree, "snaps": self.snaps, "final_commit": self.final_commit}
        self.metric_states = {k: v for k, v in self.states.items() if k in sums}
        self.scenario_metrics()
        self.history_metrics()

    def fill_record(self):
        rec, log_dir = self.rec, self.log_dir
        self.allsums = {n: s.summary() for n, s in self.states.items()}
        self.note_isolation()
        self.note_messaging()
        # the workers' numbers are the team's work; the role members' and the supervisor's replacements are the roles' (#1421)
        sums = {n: v for n, v in self.allsums.items() if n not in self.role_keys and n not in self.repl_keys}
        shas = {n: sha256_file(os.path.join(log_dir, "%s.stream.jsonl" % n)) for n in self.procs}
        self.fill_totals(sums, self.allsums)
        self.notes.append("coordination_context_tokens estimated as characters/4 (no tokenizer in the standard library)")
        self.note_roles_and_errors(sums)
        rec["outcome_checks"] = self.checks
        self.scenario_numbers(sums)
        rec["artifacts"] = {"streams": log_dir, "store_export": None,
                            "checker_report": os.path.join(log_dir, "check.json"), "stream_sha256": shas}
        self.store_metrics()
