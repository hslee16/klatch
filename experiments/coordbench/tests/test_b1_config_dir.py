"""The Claude config directory of B1 and B1-roles agents (#1520). Native cross-session messaging finds peers through the config
directory (measured: separate HOME and separate CLAUDE_CONFIG_DIR per agent breaks it; one shared config directory works with
separate HOMEs), so the agents of one B1 run share ONE config directory under the run's scratch. Every other arm keeps one per
agent. These tests prove the directory is made under the run, mode 0700, outside the per-agent directories, that the sandbox
settings and the scan treat it as the run's own for B1 agents only, and that a K or B0 agent is denied it. No model, no spend."""
import argparse
import json
import os
import shutil
import stat
import tempfile
import unittest

from helpers import BENCH  # noqa: F401
import drivers
import isolation
import runisolation
from test_messaging_verified import Harness, ProbeDriver


class Stub(runisolation.IsolationSupport):
    """The part of a RunExecution that the isolation layout reads."""

    def __init__(self, scratch, arm, names=("a", "b")):
        self.scratch, self.arm = scratch, arm
        self.log_dir = os.path.join(scratch, "..", "logs", "run")
        self.a = argparse.Namespace(klatch_bin=None)
        self.driver = drivers.FakeDriver("x")
        self.is_shared = False
        self.shared_tree = os.path.join(scratch, "shared")
        self.clones = {}
        self.origin = os.path.join(scratch, "origin.git")
        self.khome, self.kkeys = os.path.join(scratch, "klatch-home"), os.path.join(scratch, "klatch-keys")
        self.restart_dir = None
        self.init_isolation()
        for n in names:
            self.clones[n] = self.clone_path(n)
            os.makedirs(self.clones[n])
            self.agent_dir(n)


class Layout(unittest.TestCase):
    def setUp(self):
        self.tmp = os.path.realpath(tempfile.mkdtemp(prefix="coordbench-cfg-"))
        self.addCleanup(lambda: shutil.rmtree(self.tmp, ignore_errors=True))
        self.scratch = os.path.join(self.tmp, "scratch")
        os.makedirs(self.scratch)
        os.makedirs(os.path.join(self.tmp, "logs", "run"))

    def test_the_shared_directory_is_in_the_run_mode_0700_and_outside_every_agents_directory(self):
        for arm in ("B1", "B1-roles"):
            st = Stub(tempfile.mkdtemp(dir=self.tmp), arm)
            cfg = st.shared_config()
            self.assertTrue(os.path.realpath(cfg).startswith(os.path.realpath(st.scratch) + os.sep), cfg)
            self.assertEqual(stat.S_IMODE(os.stat(cfg).st_mode), 0o700)
            self.assertEqual(st.shared_config(), cfg)  # one directory, however often it is asked for
            for d in st.agent_dirs.values():
                self.assertFalse(os.path.realpath(cfg).startswith(os.path.realpath(d) + os.sep))
                self.assertFalse(os.path.exists(os.path.join(d, "claude-config")))  # no per-agent one in B1
                self.assertEqual(sorted(os.listdir(d)), ["clone", "home", "tmp"])

    def test_the_other_arms_have_no_shared_directory_and_keep_one_config_directory_per_agent(self):
        for arm in ("B0", "K1", "K2", "K3"):
            st = Stub(tempfile.mkdtemp(dir=self.tmp), arm)
            self.assertIsNone(st.shared_config())
            for d in st.agent_dirs.values():
                self.assertTrue(os.path.isdir(os.path.join(d, "claude-config")))
            self.assertEqual(len(os.listdir(st.agents_root())), 2)

    def test_the_agents_of_a_b1_run_have_separate_homes_tmp_and_clones(self):
        st = Stub(tempfile.mkdtemp(dir=self.tmp), "B1")
        ia, ib = st.iso_for("a"), st.iso_for("b")
        for kind in (0, 1, 2):  # clone, home, tmp
            self.assertNotEqual(ia["own"][kind], ib["own"][kind])
        self.assertEqual(ia["own"][3], ib["own"][3])  # the config directory is the same one
        self.assertEqual(ia["own"][3], st.shared_config())

    def test_sandbox_settings_of_a_b1_agent_allow_read_and_write_of_the_shared_directory_and_of_nothing_new(self):
        st = Stub(tempfile.mkdtemp(dir=self.tmp), "B1")
        cfg = st.shared_config()
        iso = st.iso_for("a")
        fs = isolation.sandbox_settings(dict(iso, mode="sandbox"))["sandbox"]["filesystem"]
        slash = lambda p: "//" + os.path.realpath(p).lstrip("/")
        self.assertIn(slash(cfg), fs["allowRead"])
        self.assertIn(slash(cfg), fs["allowWrite"])
        # the same set as a per-agent config directory would have given, with the shared one in its place
        own = st.agent_dir("a")
        want = {slash(p) for p in (iso["write"] + [os.path.join(own, "home"), os.path.join(own, "tmp"), st.clones["a"], cfg])}
        self.assertEqual(set(fs["allowWrite"]), want)
        for other in ("b",):
            self.assertNotIn(slash(st.agent_dir(other)), fs["allowRead"] + fs["allowWrite"])
            self.assertNotIn(slash(os.path.join(st.agent_dir(other), "home")), fs["allowRead"] + fs["allowWrite"])

    def test_a_b1_agent_is_still_denied_the_other_agents_clone_home_and_tmp_by_the_file_tool_rules(self):
        st = Stub(tempfile.mkdtemp(dir=self.tmp), "B1")
        st.shared_config()
        deny = isolation.sandbox_settings(dict(st.iso_for("a"), mode="sandbox"))["permissions"]["deny"]
        for sub in ("clone", "home", "tmp"):
            p = "//" + os.path.realpath(os.path.join(st.agent_dir("b"), sub)).lstrip("/")
            self.assertTrue([d for d in deny if d.startswith("Read(") and p in d] or
                            "Read(//%s/**)" % os.path.realpath(st.agent_dir("b")).lstrip("/") in deny, sub)
        self.assertFalse([d for d in deny if os.path.realpath(st.shared_config()).lstrip("/") in d])  # the shared one is not denied

    def test_the_scan_allows_a_b1_agent_the_shared_directory_and_flags_the_other_agents_directories(self):
        st = Stub(tempfile.mkdtemp(dir=self.tmp), "B1")
        cfg = st.shared_config()
        pol = st.policy_for("a")
        self.assertEqual(pol.kind_of(os.path.join(cfg, "projects", "x.json")), "allow")
        for sub in ("clone", "home", "tmp"):
            self.assertEqual(pol.kind_of(os.path.join(st.agent_dir("b"), sub, "f")), "guard", sub)
        n, v = isolation.scan_lines([_read(os.path.join(cfg, "settings.json")), _bash("ls " + cfg)], pol, session="a")
        self.assertEqual((n, v), (2, []))
        n, v = isolation.scan_lines([_read(os.path.join(st.agent_dir("b"), "home", ".bash_history"))], pol, session="a")
        self.assertEqual(len(v), 1)

    def test_a_b0_or_k_agent_is_denied_a_path_in_a_b1_runs_shared_directory(self):
        # the run of another arm has no such directory, but the same path in its scratch is a guarded path
        b1 = Stub(tempfile.mkdtemp(dir=self.tmp), "B1")
        cfg = b1.shared_config()
        rel = os.path.relpath(cfg, b1.scratch)
        for arm in ("B0", "K1", "K2", "K3"):
            st = Stub(tempfile.mkdtemp(dir=self.tmp), arm)
            mirror = os.path.join(st.scratch, rel)
            os.makedirs(mirror)
            pol = st.policy_for("a")
            self.assertEqual(pol.kind_of(os.path.join(mirror, "settings.json")), "guard", arm)
            n, v = isolation.scan_lines([_read(os.path.join(mirror, "settings.json"))], pol, session="a")
            self.assertEqual(len(v), 1, arm)
            deny = isolation.sandbox_settings(dict(st.iso_for("a"), mode="sandbox"))["permissions"]["deny"]
            self.assertNotIn(os.path.realpath(mirror).lstrip("/"), " ".join(deny))
            fs = isolation.sandbox_settings(dict(st.iso_for("a"), mode="sandbox"))["sandbox"]["filesystem"]
            self.assertNotIn("//" + os.path.realpath(mirror).lstrip("/"), fs["allowRead"] + fs["allowWrite"])

    def test_a_b1_agents_policy_does_not_allow_a_k_style_per_agent_config_directory_of_a_neighbour(self):
        st = Stub(tempfile.mkdtemp(dir=self.tmp), "B1")
        pol = st.policy_for("a")
        self.assertEqual(pol.kind_of(os.path.join(st.agent_dir("b"), "claude-config", "x")), "guard")


def _read(path):
    return json.dumps({"type": "assistant", "message": {"id": "m", "content": [
        {"type": "tool_use", "id": "t", "name": "Read", "input": {"file_path": path}}]}})


def _bash(cmd):
    return json.dumps({"type": "assistant", "message": {"id": "m2", "content": [
        {"type": "tool_use", "id": "t2", "name": "Bash", "input": {"command": cmd}}]}})


class InTheRun(Harness):
    """Through the whole harness: what the agent process is started with."""

    def logs_of(self, name):
        for root, _, files in os.walk(self.logs):
            if name + ".env.json" in files:
                with open(os.path.join(root, name + ".env.json")) as f:
                    return json.load(f)

    def test_b1_agents_get_one_config_directory_and_their_own_home_and_tmp(self):
        d = ProbeDriver()
        self.go(d)
        a, b = self.logs_of("a"), self.logs_of("b")
        self.assertEqual(a["cfg"], b["cfg"])
        self.assertNotEqual(a["home"], b["home"])
        self.assertNotEqual(a["tmp"], b["tmp"])
        self.assertNotEqual(a["cwd"], b["cwd"])
        self.assertNotIn(os.path.join(self.home, ".claude"), a["cfg"])
        self.assertTrue(os.path.realpath(a["cfg"]).startswith(os.path.realpath(self.scratch) + os.sep), a["cfg"])

    def test_a_b0_agents_each_have_their_own_config_directory(self):
        d = ProbeDriver()
        self.go(d, arm="B0")
        a, b = self.logs_of("a"), self.logs_of("b")
        self.assertNotEqual(a["cfg"], b["cfg"])
        self.assertTrue(a["cfg"].endswith("claude-config"))
        self.assertEqual(os.path.dirname(a["cfg"]), os.path.dirname(a["home"]))


if __name__ == "__main__":
    unittest.main()
