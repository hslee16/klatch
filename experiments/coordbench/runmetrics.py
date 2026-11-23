"""The numbers a run's record gets beyond the streams (split out of run.py, #1455): the role grants and the roles record,
the run store counts, the room seed, and the scenario metrics (git history, shared tree, recovery, history, asks). A mixin of
runexec.RunExecution: it reads the run's state from self (the record, the notes, the clones, the stream states, the kill
record). A failure is a note in the record, never a lost run."""
import json
import os
import subprocess

import asks as asks_mod
import chain
import history as history_mod
import injmetrics
import recovery
import scale as scale_mod
import scenario as scenario_mod
import shared as shared_mod
import storeexport

STREAM_SUFFIX = ".stream.jsonl"


def read_lines(path):
    with open(path, errors="replace") as f:
        return f.read().splitlines()


def resolved_models(allsums):
    models = []
    for s in allsums.values():
        models += [m for m in s["models"] if m not in models]
    return models


def tool_calls_of(sums):
    by_tool = {}
    for s in sums.values():
        for k, v in s["tool_calls"]["by_tool"].items():
            by_tool[k] = by_tool.get(k, 0) + v
    return {"total": sum(by_tool.values()), "by_tool": by_tool}


def team_metrics(sums):
    """The metrics the team's stream summaries add up to."""
    return {"git_conflicts": sum(s["git_conflicts"] for s in sums.values()),
            "messages": sum(s["messages"] for s in sums.values()),
            "human_interventions": sum(s["human_mentions"] for s in sums.values()),
            "coordination_context_tokens": sum(s["coord_context_chars"] for s in sums.values()) // 4,
            "idle_turns": sum(s["idle_turns"] for s in sums.values())}


class RunMetrics:
    # roles ---------------------------------------------------------------------------------------------------

    # the run store -------------------------------------------------------------------------------------------

    def store_metrics(self):
        """The K arms' run store as counts (#1433): artifacts.store_export, the claim and ask metrics that were null, and the
        ask counts cross-checked against the streams (scenario_metrics.store). A B arm has no store: null with a reason. A
        missing tool or an unreadable store leaves null and a note; the run is never invalidated for it."""
        rec = self.rec
        store = rec.setdefault("scenario_metrics", {}).setdefault("store", {})
        if not self.arm.startswith("K"):
            store["reason"] = "no klatch store in a %s arm (the arm has no klatch)" % self.arm
            return
        tool = self.a.bench_bin or storeexport.find_tool(self.a.klatch_bin, os.environ)
        report, why = storeexport.export(tool, self.khome, self.kkeys)
        if report is None:
            self.notes.append(why)
            store["reason"] = why
            return
        rec["artifacts"]["store_export"] = report
        m = storeexport.metrics(report)
        rec["metrics"]["claim_conflicts"] = m["claim_conflicts"]
        store.update(m)
        self.ask_cross_check(store, m)

    def ask_cross_check(self, store, m):
        try:
            streams_by_key = {n: read_lines(os.path.join(self.log_dir, n)) for n in sorted(os.listdir(self.log_dir))
                              if n.endswith(STREAM_SUFFIX)}
            ours = asks_mod.from_streams(streams_by_key, self.a.klatch_bin)
            theirs = {"created": m["asks_created"], "answered": m["asks_answered"]}
            if None in theirs.values():
                raise ValueError("the store report has no ask counts")
            store["asks_cross_check"] = asks_mod.cross_check(ours, theirs)
            if not store["asks_cross_check"]["match"]:
                self.notes.append("ask counts differ between the streams and the store: %s"
                                  % "; ".join(store["asks_cross_check"]["differences"]))
        except (OSError, ValueError) as e:
            self.notes.append("ask cross-check failed: %s" % e)
            store["asks_cross_check"] = None

    def seed_room(self, messages, env):
        """Post the fixture's seeded earlier exchange into the room, each message as its own member, before any agent starts."""
        joined = set()
        for m in messages:
            e = dict(env, KLATCH_NAME=m["as"])
            for argv in ([["join", self.a.room]] if m["as"] not in joined else []) + [["post", self.a.room, m["text"]]]:
                p = subprocess.run([self.a.klatch_bin] + argv, cwd=self.scratch, env=e, stdout=subprocess.PIPE,
                                   stderr=subprocess.STDOUT, text=True)
                if p.returncode != 0:
                    raise RuntimeError("seeding the room failed (%s): %s" % (" ".join(argv[:1]), p.stdout.strip()[-200:]))
            joined.add(m["as"])

    # the scenario ----------------------------------------------------------------------------------------------

    def scenario_metrics(self):
        """Fill duplicate_work and overwrites from git history for a fixture that has a scenario file, and keep the
        checker's extra fields under scenario_metrics. A fixture without a scenario leaves them null."""
        name = self.fx.spec.get("scenario")
        if not name:
            return
        origin, clone_dirs = (self.shared_tree, []) if self.is_shared else (self.origin, list(self.clones.values()))
        try:
            scn = scenario_mod.load(name, self.a.scenarios_dir)
            if self.fx.generated:  # a generated fixture (S7): the unit, agent and shared-file map of this size
                scn = dict(scn, **self.fx.scenario_map())
            gm = scenario_mod.git_metrics(scn, origin, clone_dirs, self.henv)
        except (scenario_mod.ScenarioError, RuntimeError, KeyError, OSError) as e:
            self.notes.append("scenario metrics failed: %s" % e)
            return
        rec = self.rec
        rec["metrics"]["duplicate_work"] = gm["duplicate_work"]
        rec["metrics"]["overwrites"] = gm["overwrites"]
        rec["scenario_metrics"] = dict(gm["details"], **scenario_mod.checker_extras(self.raw))
        self.stream_metrics(scn, origin, clone_dirs)
        if scn.get("history_check") and self.fx.topology == "T-shared":
            self.shared_metrics(scn, origin)
        elif scn.get("history_check"):
            self.single_file_overwrites(scn, origin, clone_dirs)

    def stream_metrics(self, scn, origin, clone_dirs):
        """The metrics a scenario reads from the streams: S10 injections, S4 asks, S7 scale, S3 chain, S6 recovery."""
        sm = self.rec["scenario_metrics"]
        kind = scn.get("stream_metrics")
        if kind == "injection":
            self.injection_metrics(sm)
        if scn.get("slots"):
            self.ask_metrics(scn)
        if scn.get("scale_metrics"):
            # S7: per-member coordination cost from the streams (scale.py); the store's hook bytes are added by derive()
            sm["scale"] = scale_mod.stream_parts(self.metric_states, len(self.fx.agents), self.rec["metrics"]["messages"])
        if kind == "chain":
            # S3: the blocked agents' idle and wasted turns and the handoff claim record, from the streams (chain.py)
            sm.update(chain.compute(self.log_dir, scn, self.arm, self.a.klatch_bin))
        if kind == "recovery":
            self.recovery_metrics(scn, origin, clone_dirs)

    def injection_metrics(self, sm):
        """S10 part 1: per form obeyed or not (the checker), and from the stream whether the agent saw, acted on and told its
        user about each injection (injmetrics.py)."""
        sm["obeyed_by_form"] = {i["id"]: i["obeyed"] for i in sm.get("injections", [])}
        try:
            with open(os.path.join(self.log_dir, "injection-plan.json")) as f:
                sm["injection_streams"] = injmetrics.compute(self.log_dir, json.load(f))
        except (OSError, ValueError) as e:
            self.notes.append("injection stream metrics failed: %s" % e)

    def single_file_overwrites(self, scn, origin, clone_dirs):
        """A scenario whose units share one file: overwrites are lost behaviours, found by testing every commit."""
        try:
            hist = scenario_mod.history_overwrites(scn, origin, clone_dirs, self.henv)
        except (RuntimeError, KeyError, OSError) as e:
            self.notes.append("history overwrites failed: %s" % e)
            return
        sm = self.rec["scenario_metrics"]
        self.rec["metrics"]["overwrites"] = len(hist["overwritten"])
        sm["overwritten"] = hist["overwritten"]
        sm.update(scenario_mod.overwrite_split(hist["overwritten"], self.rec["metrics"]["git_conflicts"], sm))
        for k in scn.get("not_computed", {}):
            sm.setdefault(k, None)  # named null, never omitted (the reason is in the scenario file)

    # the shared working tree (#1427) ---------------------------------------------------------------------------

    def snapshot_survivors(self, check_py, snaps):
        """{ticket: "snapshot"} for every ticket whose behaviour was present in some snapshot of the shared file."""
        ever = {}
        for text in (snaps.texts() if snaps else []):
            for t, ok in sorted(scenario_mod._ticket_pass(check_py, text, self.henv).items()):
                if ok:
                    ever.setdefault(t, "snapshot")
        return ever

    def first_text_of(self, tree, path):
        """The shared file as the first (root) commit of the tree had it; "" when it cannot be read."""
        rc, roots = shared_mod.git_out(tree, ["rev-list", "--max-parents=0", "HEAD"], self.henv)
        if rc != 0 or not roots.split():
            return ""
        rc, base = shared_mod.git_out(tree, ["show", "%s:%s" % (roots.split()[0], path)], self.henv)
        return base

    def stream_lines_by_agent(self):
        return {n[: -len(STREAM_SUFFIX)]: read_lines(os.path.join(self.log_dir, n)) for n in sorted(os.listdir(self.log_dir))
                if n.endswith(STREAM_SUFFIX)}

    def shared_overwrites(self, scn, tree, snaps):
        """(overwritten, lost-edit events): tickets present in some snapshot or lost by an agent's own file-tool edit, and
        absent from the final tree."""
        hc = scn["shared_check"]
        check_py = os.path.join(self.a.checks_dir, hc["checker"], "check.py")
        final_text = shared_mod.read_text(os.path.join(tree, hc["path"])) or ""
        final = scenario_mod._ticket_pass(check_py, final_text, self.henv)
        ever = self.snapshot_survivors(check_py, snaps)
        base = self.first_text_of(tree, hc["path"])
        events = shared_mod.lost_edit_events(self.stream_lines_by_agent(), hc["path"], base, final_text)
        owner = {v["ticket"]: k for k, v in scn["agents"].items()}
        for e in events:
            t = scn["agents"].get(e["agent"], {}).get("ticket")
            if t and not final.get(t):
                ever.setdefault(t, "stream")
        return [{"ticket": t, "author": owner.get(t), "source": ever[t]} for t in sorted(ever) if not final.get(t)], events

    def shared_metrics(self, scn, tree):
        """overwrites for the shared working tree (#1427, shared.py): tickets whose behaviour was present in some snapshot of
        the shared file, or lost by an agent's own file-tool edit (stream events), and absent from the final tree. Entries
        say which measure found them. The resolution limit is in scenario_metrics.snapshots.limit."""
        sm = self.rec["scenario_metrics"]
        info = self.shared_info
        sm["final_commit"] = info.get("final_commit")
        snaps = info.get("snaps")
        try:
            overwritten, events = self.shared_overwrites(scn, tree, snaps)
        except (RuntimeError, KeyError, OSError, subprocess.SubprocessError) as e:
            self.notes.append("shared overwrites failed: %s" % e)
            return
        self.rec["metrics"]["overwrites"] = len(overwritten)
        sm["overwritten"] = overwritten
        sm["lost_edit_events"] = events
        sm["snapshots"] = (snaps or shared_mod.Snapshots()).summary(self.a.snapshot_s)
        sm.update(scenario_mod.overwrite_split(overwritten, self.rec["metrics"]["git_conflicts"], sm))
        for k in scn.get("not_computed", {}):
            sm.setdefault(k, None)

    # S6 recovery -------------------------------------------------------------------------------------------------

    def recovery_metrics(self, scn, origin, clone_dirs):
        """The S6 metrics (recovery.py): from the streams, the timeline and the kill record of this run, and from git. A
        failure is a note, never a lost run."""
        kill = self.kill_rec
        sm = self.rec["scenario_metrics"]
        sm["kill"] = kill
        try:
            sessions = {ag: read_lines(os.path.join(self.log_dir, ag + STREAM_SUFFIX)) for ag in scn["agents"]}
            sm.update(recovery.analyze(sessions, self.timeline(), scn, self.arm, kill))
            if kill and kill.get("fired"):
                self.git_recovery(scn, kill, origin, clone_dirs)
        except (OSError, RuntimeError, KeyError, ValueError, subprocess.SubprocessError) as e:
            self.notes.append("recovery metrics failed: %s" % e)

    def timeline(self):
        tpath = os.path.join(self.log_dir, "timeline.json")
        if not os.path.exists(tpath):
            return {}
        with open(tpath) as f:
            return json.load(f)

    def git_recovery(self, scn, kill, origin, clone_dirs):
        sm = self.rec["scenario_metrics"]
        check_py = os.path.join(self.a.checks_dir, scn["recovery_check"]["checker"], "check.py")
        sm.update(recovery.git_recovery(origin, clone_dirs, scn, kill["agent"], kill.get("epoch"), self.henv, check_py))
        # false alarms seen only in git: a commit of a live agent's module by someone else
        owner = {u: ag for ag, v in scn["agents"].items() for u in v["units"]}
        have = {(d["agent"], d["unit"]) for d in sm["false_alarm_detail"]}
        for fe in sm.get("foreign_edits", []):
            if fe["unit"] != sm["victim_unit"] and (fe["author"], fe["unit"]) not in have:
                have.add((fe["author"], fe["unit"]))
                sm["false_alarm_detail"].append({"agent": fe["author"], "unit": fe["unit"], "how": "commit",
                                                 "owner": owner[fe["unit"]], "victim_unit": False})
        sm["false_alarms"] = len(sm["false_alarm_detail"])

    # S9 history and S4 asks ----------------------------------------------------------------------------------------

    def history_metrics(self):
        """The S9 metrics (history.py) for a scenario whose file has history_metrics: what the late joiner read and when, and
        the first correct contribution found by replaying its commits against the hidden checker. Kept under
        scenario_metrics.history. A failure is a note, never a lost run."""
        name = self.fx.spec.get("scenario")
        if not name:
            return
        try:
            scn = scenario_mod.load(name, self.a.scenarios_dir)
        except scenario_mod.ScenarioError:
            return
        hm = scn.get("history_metrics")
        if not hm:
            return
        agent = hm["agent"]
        rec = self.rec
        try:
            keys = sorted(k for k, n in self.agent_of.items() if n == agent)  # "dana" before "dana.r2"
            sessions = [read_lines(os.path.join(self.log_dir, k + STREAM_SUFFIX)) for k in keys]
            replayed = history_mod.replay([self.clones[agent], self.origin], agent, self.fx.checker, self.henv)
            rec.setdefault("scenario_metrics", {})["history"] = history_mod.analyze(sessions, self.a.klatch_bin, rec["arm"],
                                                                                 replayed)
            rec["scenario_metrics"]["history"]["replayed"] = [
                {"sha": r["sha"], "correct": r["correct"], "facts": r.get("facts")} for r in replayed]
        except (OSError, RuntimeError, KeyError, ValueError, subprocess.SubprocessError) as e:
            self.notes.append("history metrics failed: %s" % e)

    def ask_metrics(self, scn):
        """The S4 ask and answer metrics from the agents' message events (asks.py); the checker's slot states give the
        guesses and unanswered dependencies. Skipped, with a note, when the checker's slots are not there."""
        slots = self.rec["scenario_metrics"].get("slots")
        if not isinstance(slots, dict):
            self.notes.append("ask metrics skipped: the checker printed no slots")
            return
        try:
            with open(os.path.join(self.fx.checks_dir, self.fx.spec["checker"], "facts.json")) as f:
                facts = json.load(f)["facts"]
        except (OSError, ValueError, KeyError) as e:
            self.notes.append("ask metrics skipped: no facts for the checker (%s)" % e)
            return
        states = self.metric_states
        events = sorted(([dict(e, agent=n) for n, s in states.items() for e in s.msg_events]),
                        key=lambda e: (e["t"] if e["t"] is not None else 0))
        done = {n: s.done_t for n, s in states.items() if s.done_t is not None}
        self.rec["scenario_metrics"]["asks"] = asks_mod.compute(events, done, scn, facts, slots)
