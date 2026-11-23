"""The role launcher (#1421): arms K3 and B1-roles start an integrator and a supervisor besides the workers, each its own
agent process in the run, with its own cost line in the run record. Fake agents only: no model, no spend.

K3 needs a built klatch (KLATCH_BIN, and KLATCH_BENCH_BIN for the store export): the roles are granted in the run store.
B1-roles needs no binary.
"""
import json
import os
import re
import subprocess
import unittest
from unittest import mock

from helpers import BENCH  # noqa: F401
import drivers
import report
import roles
import run
import schema
from test_run_fake import Base, KLATCH_BIN, NEEDS_BIN, read_json, files_under
from test_s1_scenario import steps as s1_steps, TICKETS
from test_s6_scenario import FIXTURE as S6_FIXTURE

ARMS = os.path.join(BENCH, "arms")
# the visible tests of the fixture need both modules, so a half-merged main would fail them: the integrator checks it imports
TRIVIAL_TEST = ["python3", "-c", "import alpha, beta"]
JOIN = {"op": "klatch", "args": ["join", "{{ROOM}}"]}


def a_steps(name, fn):
    return [{"op": "write", "path": fn + ".py", "content": "def %s():\n    return '%s-done'\n" % (fn, fn)},
            {"op": "sync", "message": "%s: implement %s()" % (fn, fn)}, {"op": "final", "text": "DONE " + name}]


def trivial_team(**over):
    """a and b publish through sync (a branch and ready: in a role arm), an integrator merges both in order, a supervisor
    does nothing."""
    ag = {"a": a_steps("a", "alpha"), "b": a_steps("b", "beta"),
          "integrator": [{"op": "integrate", "branches": ["a", "b"], "test": TRIVIAL_TEST},
                         {"op": "final", "text": "DONE integrator"}],
          "supervisor": [{"op": "say", "text": "all quiet"}, {"op": "final", "text": "DONE supervisor"}]}
    ag.update(over)
    return ag


class RoleBase(Base):
    fixture = "trivial"

    def argv(self, arms="B1-roles", runs=1, **kw):
        a = super().argv(arms, runs, **kw)
        a[a.index("--fixture") + 1] = self.fixture
        a += ["--roles-grace-s", "20"]
        return a

    def records(self, arm=None):
        out = []
        for d, _, names in os.walk(os.path.join(self.results, "t1")):
            for n in names:
                if n.endswith(".json") and n != "suite.json" and (arm is None or os.path.basename(d) == arm):
                    out.append(read_json(os.path.join(d, n)))
        return sorted(out, key=lambda r: r["run_id"])

    def write_script(self, agents):
        path = os.path.join(self.tmp, "fake-roles-%d.json" % len(os.listdir(self.tmp)))
        with open(path, "w") as f:
            json.dump({"agents": agents}, f)
        return path

    def one(self, arm, agents, expect_code=None, **kw):
        code, out, err = self.go(self.argv(arm, 1, fake_script=self.write_script(agents), **kw))
        recs = self.records()
        self.assertEqual(len(recs), 1, out + err)
        r = recs[0]
        self.assertEqual(schema.validate_record(r), [], r["run_id"])
        if expect_code is not None:
            self.assertEqual(code, expect_code, out + err)
        return r

    def stream(self, rec, key):
        with open(os.path.join(self.logs, rec["run_id"], key + ".stream.jsonl")) as f:
            return f.read()


class TestPrompts(unittest.TestCase):
    def build(self, arm, name="a"):
        return run.build_prompt(ARMS, arm, name=name, team=["a", "b"], task="DO THE THING", workdir="/w/" + name,
                                test_command="python3 -m unittest", klatch_bin="/opt/k/klatch", room="lobby",
                                max_turns=30, max_minutes=20)

    def test_a_role_arm_worker_gets_the_base_arm_text_and_its_own_section_only(self):
        for arm, base, marker in (("K3", "K2", "/opt/k/klatch"), ("B1-roles", "B1", "SendMessage")):
            text = self.build(arm)
            self.assertIn("ready: BRANCH", text)
            self.assertNotIn("You are `integrator`", text, arm)
            self.assertNotIn("You are `supervisor`", text, arm)
            self.assertNotIn("{{", text)
            self.assertIn(marker, text)
            self.assertIn("Telling others what you are doing:", text, "the base arm text is part of the worker prompt")
            self.assertGreater(len(text), len(self.build(base)))

    def test_the_role_prompts_are_the_arm_file_sections_filled(self):
        for arm in ("K3", "B1-roles"):
            for role in roles.ROLES:
                text = roles.role_prompt(ARMS, arm, role, name=role, workers=["a", "b"], workdir="/w/" + role,
                                         test_command="T-CMD", klatch_bin="/opt/k/klatch", room="lobby",
                                         restart_command="python3 /x/restart_member.py", max_turns=30, max_minutes=20)
                self.assertIn("You are `%s`" % role, text)
                self.assertNotIn("{{", text)
                self.assertNotIn("<!--", text)
                other = "supervisor" if role == "integrator" else "integrator"
                self.assertNotIn("You are `%s`" % other, text)
                self.assertIn("/w/%s" % role, text)
                self.assertIn("a, b", text)
        k3 = roles.role_prompt(ARMS, "K3", "integrator", "integrator", ["a"], "/w", "T-CMD", "/opt/k/klatch", "lobby", "", 30, 20)
        self.assertIn("/opt/k/klatch wait lobby", k3)
        self.assertIn("T-CMD", k3)
        sup = roles.role_prompt(ARMS, "B1-roles", "supervisor", "supervisor", ["a"], "/w", "T", None, "lobby",
                                "python3 /x/restart_member.py", 30, 20)
        self.assertIn("python3 /x/restart_member.py", sup)
        for word in ("klatch", "lobby"):
            self.assertNotIn(word, sup.lower(), "no klatch word in a B-arm role prompt")

    def test_the_two_role_arms_get_the_same_frame_around_their_sections(self):
        frames = []
        for arm in ("K3", "B1-roles"):
            text = roles.role_prompt(ARMS, arm, "integrator", "integrator", ["a", "b"], "/w", "T", "/k", "lobby", "", 30, 20)
            frames.append(text.split("You are `integrator`")[0])
        self.assertEqual(frames[0], frames[1])


class TestDriverCommands(unittest.TestCase):
    def setUp(self):
        self.d = drivers.ClaudeDriver("claude", "model-x", 40, 1.0, klatch_bin="/opt/k/klatch")

    def cmd(self, arm, **kw):
        argv, _ = self.d.command("integrator", arm, "PROMPT", "/w", "/out", **kw)
        return argv

    def value(self, argv, flag):
        return argv[argv.index(flag) + 1]

    def test_a_role_member_cannot_edit_files_and_a_worker_can(self):
        role = self.cmd("K3", role="integrator")
        tools = role[role.index("--allowedTools") + 1:role.index("--max-budget-usd")]
        for t in ("Edit", "Write"):
            self.assertNotIn(t, tools)
        self.assertIn("Read", tools)
        self.assertIn("Bash(/opt/k/klatch:*)", tools)
        worker = self.cmd("K3")
        self.assertIn("Edit", worker)

    def test_a_role_member_has_its_own_budget_model_and_turn_cap(self):
        argv = self.cmd("K3", role="supervisor", max_usd=0.4, model="model-y", max_turns=12)
        self.assertEqual(self.value(argv, "--max-budget-usd"), "0.40")
        self.assertEqual(self.value(argv, "--model"), "model-y")
        self.assertEqual(self.value(argv, "--max-turns"), "12")
        self.assertEqual(self.value(self.cmd("K3"), "--model"), "model-x")
        self.assertEqual(self.value(self.cmd("K3"), "--max-budget-usd"), "1.00")

    def test_b1_roles_has_messaging_and_sleep_like_b1_and_k3_has_neither(self):
        b = self.cmd("B1-roles", role="integrator")
        self.assertIn("SendMessage", b)
        self.assertIn("ListAgents", b)
        self.assertIn("Bash(sleep:*)", b)
        self.assertEqual(json.loads(self.value(b, "--settings")), {"crossSessionInbound": "accept"})
        k = self.cmd("K3", role="integrator")
        self.assertNotIn("SendMessage", k[k.index("--allowedTools"):k.index("--max-budget-usd")])
        self.assertEqual(json.loads(self.value(k, "--settings"))["crossSessionInbound"], "refuse")
        self.assertNotIn("Bash(/opt/k/klatch:*)", self.cmd("B1-roles", role="supervisor"))

    def test_the_fake_driver_names_the_role(self):
        fd = drivers.FakeDriver("/s.json", "/k", "lobby")
        argv, _ = fd.command("supervisor", "K3", "P", "/w", "/out", role="supervisor")
        self.assertEqual(argv[argv.index("--role") + 1], "supervisor")
        argv, _ = fd.command("a", "K3", "P", "/w", "/out")
        self.assertNotIn("--role", argv)


class TestRestartScript(unittest.TestCase):
    def test_a_request_is_a_file_the_harness_answers(self):
        import tempfile
        import threading
        d = tempfile.mkdtemp(prefix="rs-")
        self.addCleanup(lambda: __import__("shutil").rmtree(d, ignore_errors=True))
        with open(os.path.join(d, "workers"), "w") as f:
            f.write("a\nc\n")
        script = os.path.join(BENCH, "restart_member.py")
        env = dict(os.environ, COORDBENCH_RESTART_DIR=d)

        def answer():
            import time
            for _ in range(100):
                if os.path.exists(os.path.join(d, "c.request")):
                    with open(os.path.join(d, "c.result"), "w") as f:
                        f.write("started: c-2\n")
                    return
                time.sleep(0.05)
        t = threading.Thread(target=answer)
        t.start()
        p = subprocess.run(["python3", script, "c", "--wait-s", "10"], env=env, stdout=subprocess.PIPE, text=True)
        t.join()
        self.assertEqual(p.returncode, 0, p.stdout)
        self.assertIn("started: c-2", p.stdout)

    def test_a_name_that_is_not_a_worker_is_refused_without_a_request(self):
        import tempfile
        d = tempfile.mkdtemp(prefix="rs-")
        self.addCleanup(lambda: __import__("shutil").rmtree(d, ignore_errors=True))
        with open(os.path.join(d, "workers"), "w") as f:
            f.write("a\nc\n")
        script = os.path.join(BENCH, "restart_member.py")
        for argv in (["zed"], ["../x"], []):
            p = subprocess.run(["python3", script] + argv, env=dict(os.environ, COORDBENCH_RESTART_DIR=d),
                               stdout=subprocess.PIPE, text=True)
            self.assertEqual(p.returncode, 2, p.stdout)
            self.assertIn("a, c", p.stdout)
        self.assertEqual(sorted(os.listdir(d)), ["workers"])


class TestSettings(unittest.TestCase):
    def test_the_scenario_files_allow_both_role_arms_with_role_settings(self):
        import scenario
        for name in ("s1", "s2", "s6"):
            s = scenario.load(name)
            self.assertIn("K3", s["arms"], name)
            self.assertIn("B1-roles", s["arms"], name)
            self.assertNotIn("B1-roles", s.get("arms_at_m0", []), name)
            cfg = s["roles"]
            self.assertEqual(cfg["arms"], ["K3", "B1-roles"])
            self.assertEqual(cfg["members"], ["integrator", "supervisor"])
            self.assertIn("model", cfg)
            self.assertGreater(cfg["per_role_usd"], 0)

    def test_settings_resolve_from_the_scenario_then_the_command_line(self):
        class A:
            roles = None
            roles_model = None
            role_usd = None
            roles_grace_s = None
            model = "agent-model"
            max_run_usd = 3.0
        scn = {"roles": {"arms": ["K3", "B1-roles"], "members": ["integrator", "supervisor"], "model": "role-model",
                         "per_role_usd": 0.25, "grace_s": 90, "max_restarts": 2}}
        cfg = roles.resolve(A(), scn, "K3", n_workers=4)
        self.assertEqual((cfg["members"], cfg["model"], cfg["per_role_usd"], cfg["grace_s"], cfg["max_restarts"]),
                         (["integrator", "supervisor"], "role-model", 0.25, 90, 2))
        a = A()
        a.roles, a.roles_model, a.role_usd, a.roles_grace_s = "integrator", "other", 0.5, 5
        cfg = roles.resolve(a, scn, "B1-roles", n_workers=4)
        self.assertEqual((cfg["members"], cfg["model"], cfg["per_role_usd"], cfg["grace_s"]), (["integrator"], "other", 0.5, 5))
        self.assertIsNone(roles.resolve(A(), scn, "K2", n_workers=4))
        self.assertIsNone(roles.resolve(A(), scn, "B1", n_workers=4))
        cfg = roles.resolve(A(), None, "K3", n_workers=2)  # a fixture with no scenario file: both roles, the agent model
        self.assertEqual((cfg["members"], cfg["model"]), (["integrator", "supervisor"], "agent-model"))
        self.assertAlmostEqual(cfg["per_role_usd"], 0.75)  # the run cap (3.0) spread over every member (2 workers, 2 roles)
        with self.assertRaises(ValueError):
            a.roles = "treasurer"
            roles.resolve(a, scn, "K3", n_workers=2)


class TestB1RolesEndToEnd(RoleBase):
    def test_the_trivial_fixture_runs_with_a_fake_integrator_and_supervisor(self):
        r = self.one("B1-roles", trivial_team(), expect_code=0)
        self.assertEqual((r["arm"], r["outcome"], r["valid"]), ("B1-roles", "ok", True))
        self.assertEqual((r["metrics"]["units_total"], r["metrics"]["units_completed"]), (2, 2))
        self.assertEqual(sorted(r["turns"]["per_agent"]), ["a", "b"], "roles are not in the workers' turns")
        self.assertEqual(sorted(r["roles"]["members"]), ["integrator", "supervisor"])
        for name in roles.ROLES:
            m = r["roles"]["members"][name]
            self.assertEqual((m["state"], m["exit"]), ("ok", 0), name)
            self.assertGreater(m["turns"], 0)
            self.assertGreaterEqual(m["wall_s"], 0)
            self.assertIn("input", m["tokens"])
        self.assertIsNone(r["roles"]["members"]["integrator"].get("granted"), "no store in a B arm")
        # the board and the messaging of the B1 baseline are there for the roles too
        self.assertIn("SendMessage", self.stream(r, "integrator"))
        self.assertTrue(any(p.endswith("integrator.prompt.md") for p in files_under(self.logs)))
        self.assertTrue(any(p.endswith("supervisor.stream.jsonl") for p in files_under(self.logs)))

    def test_the_workers_publish_a_branch_and_only_the_integrator_pushes_main(self):
        r = self.one("B1-roles", trivial_team())
        for who in ("a", "b"):
            text = self.stream(r, who)
            self.assertIn("refs/heads/%s" % who, text)
            self.assertNotIn("HEAD:main", text, "a worker does not push to main in a role arm")
            self.assertIn("ready: %s" % who, text)
        self.assertIn("HEAD:main", self.stream(r, "integrator"))
        self.assertIn("merged: a", self.stream(r, "integrator"))

    def test_the_integrator_merges_in_order(self):
        r = self.one("B1-roles", trivial_team(integrator=[
            {"op": "integrate", "branches": ["b", "a"], "test": TRIVIAL_TEST}, {"op": "final", "text": "DONE integrator"}]))
        text = self.stream(r, "integrator")
        self.assertLess(text.index("merged: b"), text.index("merged: a"))
        self.assertEqual(r["metrics"]["units_completed"], 2)

    def test_a_branch_that_fails_the_tests_is_dropped_and_the_worker_does_not_finish(self):
        ag = trivial_team(integrator=[{"op": "integrate", "branches": ["a", "b"], "test": ["false"]},
                                      {"op": "final", "text": "DONE integrator"}],
                          a=[{"op": "write", "path": "alpha.py", "content": "def alpha():\n    return 'alpha-done'\n"},
                             {"op": "sync", "message": "alpha", "merge_wait_s": 3}],
                          b=[{"op": "write", "path": "beta.py", "content": "def beta():\n    return 'beta-done'\n"},
                             {"op": "sync", "message": "beta", "merge_wait_s": 3}])
        r = self.one("B1-roles", ag)
        self.assertIn("dropped: a checks-failed", self.stream(r, "integrator"))
        self.assertEqual(r["metrics"]["units_completed"], 0)
        self.assertEqual(r["outcome"], "crash", "the workers that were never merged ended with an error")
        self.assertTrue(r["valid"])

    def test_role_costs_are_recorded_apart_and_added_to_the_total(self):
        ag = trivial_team()
        ag["a"] = [{"op": "cost", "usd": 0.05}] + ag["a"]
        ag["b"] = [{"op": "cost", "usd": 0.05}] + ag["b"]
        ag["integrator"] = [{"op": "cost", "usd": 0.2}] + ag["integrator"]
        ag["supervisor"] = [{"op": "cost", "usd": 0.1}] + ag["supervisor"]
        r = self.one("B1-roles", ag)
        self.assertAlmostEqual(r["roles"]["members"]["integrator"]["cost_usd"], 0.2)
        self.assertAlmostEqual(r["roles"]["members"]["supervisor"]["cost_usd"], 0.1)
        self.assertAlmostEqual(r["spend"]["roles_usd"], 0.3)
        self.assertAlmostEqual(r["spend"]["workers_usd"], 0.1)
        self.assertAlmostEqual(r["spend"]["usd"], 0.4, msg="the team total includes the roles")
        meta = read_json(os.path.join(self.results, "t1", "suite.json"))
        self.assertAlmostEqual(meta["spent_usd"], 0.4, msg="the suite cap sees the roles")
        with open(os.path.join(self.results, "LEDGER.md")) as f:
            self.assertIn("0.4000", f.read())

    def test_a_role_that_spends_past_the_run_cap_ends_the_run_as_spend_cap(self):
        ag = trivial_team(supervisor=[{"op": "cost", "usd": 0.9}, {"op": "hang"}])
        r = self.one("B1-roles", ag, max_run_usd=0.5)
        self.assertEqual(r["outcome"], "spend-cap")
        self.assertGreaterEqual(r["spend"]["usd"], 0.9)
        self.assertGreaterEqual(r["spend"]["roles_usd"], 0.9)

    def test_the_suite_cap_counts_the_roles(self):
        ag = trivial_team(supervisor=[{"op": "cost", "usd": 0.9}, {"op": "final", "text": "DONE supervisor"}])
        code, out, err = self.go(self.argv("B1-roles", 5, fake_script=self.write_script(ag), max_run_usd=1.0, max_suite_usd=2.5))
        self.assertEqual(code, 3, out + err)
        self.assertEqual(len(self.records()), 2)

    def test_a_failing_role_member_is_recorded_and_does_not_invalidate_the_run(self):
        r = self.one("B1-roles", trivial_team(supervisor=[{"op": "say", "text": "starting"}, {"op": "exit", "code": 3}]),
                     expect_code=0)
        sup = r["roles"]["members"]["supervisor"]
        self.assertEqual((sup["state"], sup["exit"]), ("failed", 3))
        self.assertEqual(r["roles"]["members"]["integrator"]["state"], "ok")
        self.assertTrue(r["valid"])
        self.assertEqual(r["outcome"], "ok", "the outcome is the workers'")
        self.assertEqual(r["metrics"]["units_completed"], 2)
        self.assertIn("supervisor", r["notes"])
        self.assertIn("exit", r["notes"])

    def test_a_role_that_never_stops_is_ended_after_the_grace_and_recorded(self):
        r = self.one("B1-roles", trivial_team(supervisor=[{"op": "say", "text": "watching"}, {"op": "hang"}]),
                     roles_grace_s=2)
        sup = r["roles"]["members"]["supervisor"]
        self.assertEqual(sup["state"], "stopped-by-harness")
        self.assertTrue(r["valid"])
        self.assertEqual(r["outcome"], "ok")
        self.assertLess(r["wall_s"], 40)

    def test_the_arm_runs_beside_the_base_arms_and_leaves_them_alone(self):
        code, out, err = self.go(self.argv("B1,B1-roles", 1, fake_script=self.write_script(trivial_team())))
        recs = {r["arm"]: r for r in self.records()}
        self.assertEqual(sorted(recs), ["B1", "B1-roles"])
        self.assertIsNone(recs["B1"]["roles"])
        self.assertNotIn("roles_usd", recs["B1"]["spend"])
        self.assertEqual(recs["B1"]["models"]["roles"], None)

    def test_models_records_the_roles_model(self):
        r = self.one("B1-roles", trivial_team(), roles_model="role-model-x")
        self.assertEqual(r["models"]["roles"], "role-model-x")
        self.assertEqual(r["roles"]["model"], "role-model-x")

    def test_roles_can_be_chosen_on_the_command_line(self):
        r = self.one("B1-roles", trivial_team(integrator=[{"op": "integrate", "branches": ["a", "b"], "test": TRIVIAL_TEST}]),
                     roles="integrator")
        self.assertEqual(sorted(r["roles"]["members"]), ["integrator"])
        self.assertFalse(any(p.endswith("supervisor.stream.jsonl") for p in files_under(self.logs)))

    def test_a_worker_may_not_be_named_like_a_role(self):
        fx = os.path.join(self.tmp, "fixtures")
        import shutil
        shutil.copytree(os.path.join(BENCH, "fixtures", "trivial"), os.path.join(fx, "trivial"))
        spec = read_json(os.path.join(fx, "trivial", "fixture.json"))
        spec["agents"][0]["name"] = "integrator"
        with open(os.path.join(fx, "trivial", "fixture.json"), "w") as f:
            json.dump(spec, f)
        self.go(self.argv("B1-roles", 1, fixtures_dir=fx, fake_script=self.write_script(trivial_team())))
        r = self.records()[0]
        self.assertEqual((r["outcome"], r["valid"]), ("error", False))
        self.assertIn("integrator", r["notes"])


class TestReportShowsRoles(unittest.TestCase):
    def test_the_report_has_the_roles_cost_beside_the_total(self):
        from helpers import make_record
        recs = []
        for i in range(3):
            r = make_record(arm="K3", run="r%02d" % i, cost=0.5)
            r["spend"].update(workers_usd=0.3, roles_usd=0.2)
            recs.append(r)
            recs.append(make_record(arm="K2", run="r%02d" % i, cost=0.3))
        a = report.analyse(recs, resamples=200)
        cell = a["scenarios"]["trivial"]["arms"]["K3"]
        self.assertAlmostEqual(cell["metrics"]["roles_cost_usd"]["median"], 0.2)
        self.assertAlmostEqual(cell["metrics"]["cost_usd"]["median"], 0.5)
        self.assertIsNone(a["scenarios"]["trivial"]["arms"]["K2"]["metrics"].get("roles_cost_usd"))
        self.assertIn("roles_cost_usd", report.render_markdown(a))


class TestSchema(unittest.TestCase):
    def test_the_roles_key_is_optional_but_checked_when_present(self):
        from helpers import make_record
        r = make_record(arm="K3")
        self.assertEqual(schema.validate_record(r), [])
        r["roles"] = {"model": None, "members": {"integrator": {"state": "ok"}}}
        self.assertTrue(schema.validate_record(r))  # a member needs its numbers
        r["roles"] = None
        self.assertEqual(schema.validate_record(r), [])
        r["roles"] = {"model": None, "members": {"integrator": {"turns": 1, "tokens": {}, "cost_usd": 0.1,
                                                                "wall_s": 1.0, "exit": 0, "state": "ok"}},
                      "replacements": {}, "restarts": []}
        self.assertEqual(schema.validate_record(r), [])


@NEEDS_BIN
class TestK3EndToEnd(RoleBase):
    def argv(self, arms="K3", runs=1, **kw):
        return super().argv(arms, runs, **kw)

    def k3_team(self, **over):
        ag = trivial_team()
        for who in ("a", "b"):
            ag[who] = [JOIN, {"op": "klatch", "args": ["post", "{{ROOM}}", "working: " + who]}] + ag[who]
        ag["a"].insert(2, {"op": "klatch", "args": ["claim", "{{ROOM}}", "merge:main"], "allow_fail": True})
        ag["integrator"] = [JOIN, {"op": "klatch", "args": ["claim", "{{ROOM}}", "merge:main"]}] + ag["integrator"][:-1] + [
            {"op": "klatch", "args": ["release", "{{ROOM}}", "merge:main"], "allow_fail": True}, ag["integrator"][-1]]
        ag["supervisor"] = [JOIN] + ag["supervisor"]
        ag.update(over)
        return ag

    def test_the_trivial_fixture_runs_end_to_end_with_the_integrator_granted_its_role(self):
        r = self.one("K3", self.k3_team(), expect_code=0)
        self.assertEqual((r["arm"], r["outcome"], r["valid"]), ("K3", "ok", True), r["notes"])
        self.assertEqual(r["metrics"]["units_completed"], 2)
        integ = r["roles"]["members"]["integrator"]
        self.assertTrue(integ["granted"], r["roles"])
        self.assertEqual((integ["state"], integ["exit"]), ("ok", 0))
        self.assertEqual(r["roles"]["members"]["supervisor"]["granted"], None, "klatch has no supervisor role")
        text = self.stream(r, "integrator")
        self.assertRegex(text, r"claimed[^\n]*merge:main")
        self.assertNotRegex(self.stream(r, "a"), r"claimed[^\\\"]*merge:main", "a worker without the role is refused the claim")
        self.assertIn("merged: a", text)
        for who in ("a", "b"):
            self.assertNotIn("HEAD:main", self.stream(r, who))

    def test_the_run_store_is_the_harness_own_and_the_role_grant_is_in_its_audit_only(self):
        with mock.patch.object(roles.secrets, "token_urlsafe", return_value="SENTINEL-ADMIN-PASS"):
            r = self.one("K3", self.k3_team())
        export = r["artifacts"]["store_export"]
        self.assertIsNotNone(export, r["notes"])
        self.assertGreaterEqual(r["scenario_metrics"]["store"]["claims_granted"], 1, "the integrator's claim is in the run store")
        for root in (self.results, self.logs):
            for p in files_under(root):
                with open(p, errors="replace") as f:
                    self.assertNotIn("SENTINEL-ADMIN-PASS", f.read(), p)
        self.assertEqual(os.listdir(self.fakehome), [], "nothing was written under the user's home")

    def test_the_integrator_merges_in_order_and_main_history_shows_it(self):
        r = self.one("K3", self.k3_team(integrator=[
            JOIN, {"op": "klatch", "args": ["claim", "{{ROOM}}", "merge:main"]},
            {"op": "integrate", "branches": ["b", "a"], "test": TRIVIAL_TEST}, {"op": "final", "text": "DONE integrator"}]))
        text = self.stream(r, "integrator")
        self.assertLess(text.index("merged: b"), text.index("merged: a"))
        self.assertEqual(r["metrics"]["units_completed"], 2)

    def test_a_failing_role_member_is_recorded_in_k3_too(self):
        r = self.one("K3", self.k3_team(supervisor=[JOIN, {"op": "exit", "code": 4}]))
        sup = r["roles"]["members"]["supervisor"]
        self.assertEqual((sup["state"], sup["exit"]), ("failed", 4))
        self.assertTrue(r["valid"])
        self.assertEqual(r["metrics"]["units_completed"], 2)

    def test_a_role_that_cannot_be_granted_is_recorded_and_the_run_goes_on(self):
        with mock.patch.object(roles, "grant_integrator", return_value=(False, "refused for the test")):
            r = self.one("K3", self.k3_team(integrator=[JOIN, {"op": "klatch", "args": ["claim", "{{ROOM}}", "merge:main"],
                                                              "allow_fail": True},
                                                       {"op": "integrate", "branches": ["a", "b"], "test": TRIVIAL_TEST},
                                                       {"op": "final", "text": "DONE integrator"}]))
        integ = r["roles"]["members"]["integrator"]
        self.assertFalse(integ["granted"])
        self.assertIn("refused for the test", integ["grant_error"])
        self.assertTrue(r["valid"])
        self.assertIn("integrator", r["notes"])


@NEEDS_BIN
class TestK3OnS1(RoleBase):
    fixture = "tallykit"

    def test_four_workers_one_integrator_merge_everything(self):
        ag = {}
        for n in "abcd":
            ag[n] = [JOIN] + s1_steps(n)
        ag["integrator"] = [JOIN, {"op": "klatch", "args": ["claim", "{{ROOM}}", "merge:main"]},
                            {"op": "integrate", "branches": list("abcd"), "resolve": "union",
                             "test": ["python3", "-c", "import tallykit"]},
                            {"op": "final", "text": "DONE integrator"}]
        ag["supervisor"] = [JOIN, {"op": "final", "text": "DONE supervisor"}]
        r = self.one("K3", ag, expect_code=0)
        self.assertEqual(r["outcome"], "ok", r["notes"])
        self.assertEqual((r["metrics"]["units_total"], r["metrics"]["units_completed"]), (8, 8))
        self.assertEqual(sorted(r["turns"]["per_agent"]), ["a", "b", "c", "d"])
        self.assertEqual(r["scenario_metrics"]["merge_state"], "clean")
        self.assertTrue(r["roles"]["members"]["integrator"]["granted"])
        self.assertEqual(r["metrics"]["duplicate_work"], 0)


def s6_team():
    fake = read_json(os.path.join(S6_FIXTURE, "fake.json"))["agents"]
    ag = {k: [dict(s, **({"from": os.path.join(S6_FIXTURE, s["from"])} if s.get("op") == "copy_from" else {})) for s in v]
          for k, v in fake.items()}  # the script is not in the fixture directory: make the reference paths absolute
    ref = os.path.join(S6_FIXTURE, "reference", "recoverkit", "bits.py")
    ag["integrator"] = [JOIN, {"op": "klatch", "args": ["claim", "{{ROOM}}", "merge:main"]},
                        {"op": "integrate", "branches": ["b", "c-2", "a"], "resolve": "union", "timeout": 60,
                         "test": ["true"]},
                        {"op": "final", "text": "DONE integrator"}]
    ag["supervisor"] = [JOIN, {"op": "wait_file", "path": "kill.json", "timeout": 60},
                        {"op": "restart", "member": "c"}, {"op": "final", "text": "DONE supervisor"}]
    ag["c"] = [dict(x, direct=True) if x.get("op") == "sync" else x for x in ag["c"]]
    ag["c-2"] = [JOIN, {"op": "copy_from", "from": ref, "path": "recoverkit/bits.py"},
                 {"op": "sync", "message": "bits, finished by the replacement", "resolve": "union"},
                 {"op": "final", "text": "bits finished. DONE c-2"}]
    # the survivors do not take over: the supervisor's replacement does the module
    ag["b"] = [s for s in ag["b"] if s.get("op") != "wait_file"][:4] + [{"op": "final", "text": "DONE b"}]
    ag["b"] = [s for s in ag["b"] if not (s.get("op") == "copy_from" and "bits" in s["from"])]
    ag["a"] = [s for s in ag["a"] if s.get("op") != "klatch" or "post" not in s["args"]]
    return ag


@NEEDS_BIN
class TestK3SupervisorRestartsTheKilledWorkerOnS6(RoleBase):
    fixture = "recovery"

    def test_the_supervisor_restarts_the_killed_worker_and_the_s6_metrics_see_the_recovery(self):
        r = self.one("K3", s6_team())
        sm = r["scenario_metrics"]
        self.assertTrue(sm["kill"]["fired"], r["notes"])
        self.assertEqual(sm["victim"], "c")
        self.assertEqual(r["roles"]["restarts"], [{"member": "c", "replacement": "c-2", "started": True, "reason": None}])
        self.assertIn("c-2", r["roles"]["replacements"])
        self.assertEqual(sm["recovered_by_commit_author"], "c-2", sm)
        self.assertIsNotNone(sm["time_to_module_recovery_s"])
        self.assertEqual(sm["false_alarms"], 0)
        ids = {c["id"]: c["pass"] for c in r["outcome_checks"]}
        self.assertTrue(ids.get("bits"), ids)
        self.assertTrue(r["valid"], r["notes"])
        self.assertGreater(r["spend"].get("roles_usd", 0) + 1, 0)

    def test_no_restart_happens_without_a_request_and_one_cannot_be_asked_twice(self):
        ag = s6_team()
        ag["supervisor"] = [JOIN, {"op": "wait_file", "path": "kill.json", "timeout": 60},
                            {"op": "restart", "member": "c"}, {"op": "restart", "member": "c", "allow_fail": True},
                            {"op": "final", "text": "DONE supervisor"}]
        r = self.one("K3", ag)
        started = [x for x in r["roles"]["restarts"] if x["started"]]
        refused = [x for x in r["roles"]["restarts"] if not x["started"]]
        self.assertEqual(len(started), 1)
        self.assertEqual(len(refused), 1)
        self.assertIn("limit", refused[0]["reason"])


class TestB1RolesOnS6(RoleBase):
    fixture = "recovery"

    def test_a_replacement_started_by_the_b1_roles_supervisor_recovers_the_module(self):
        ag = s6_team()
        ag["supervisor"] = [{"op": "wait_file", "path": "kill.json", "timeout": 60}, {"op": "restart", "member": "c"},
                            {"op": "final", "text": "DONE supervisor"}]
        ag["integrator"] = [s for s in ag["integrator"] if s.get("op") not in ("klatch",)]
        for who in ag:
            ag[who] = [s for s in ag[who] if s.get("op") != "klatch"]
        ag["c"] = [s for s in ag["c"] if s.get("op") != "klatch"]
        r = self.one("B1-roles", ag)
        sm = r["scenario_metrics"]
        self.assertTrue(sm["kill"]["fired"], r["notes"])
        self.assertEqual(sm["recovered_by_commit_author"], "c-2", sm)
        self.assertEqual(r["roles"]["restarts"][0]["started"], True)
        self.assertEqual(r["roles"]["restarts"][0]["replacement"], "c-2")


if __name__ == "__main__":
    unittest.main()
