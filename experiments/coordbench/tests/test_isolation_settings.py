"""The settings the claude driver generates to keep an agent to its own directories (#1519). Nothing here starts claude."""
import json
import os
import tempfile
import unittest

from helpers import BENCH  # noqa: F401
import drivers


def iso(root, mode="sandbox"):
    j = lambda *p: os.path.join(root, *p)
    return {"mode": mode, "root": j("scratch"), "own": [j("scratch", "ta", "clone"), j("scratch", "ta", "home"), j("scratch", "ta", "tmp")],
            "write": [j("scratch", "origin.git"), j("scratch", "klatch-home")], "read": [j("bin")],
            "guard": [j("scratch"), j("logs"), j("repo")], "others": [j("scratch", "tb")], "settings_dir": j("scratch", "ta")}


class Settings(unittest.TestCase):
    def setUp(self):
        self.root = os.path.realpath(tempfile.mkdtemp())
        self.addCleanup(lambda: __import__("shutil").rmtree(self.root, ignore_errors=True))
        os.makedirs(os.path.join(self.root, "scratch", "ta"))
        os.makedirs(os.path.join(self.root, "logs"))
        self.d = drivers.ClaudeDriver("claude", "m", 10, 1.0, "/b/klatch")

    def command(self, arm="K2", mode="sandbox"):
        argv, _ = self.d.command("a", arm, "P", "/c", os.path.join(self.root, "logs"), isolation=iso(self.root, mode))
        return argv

    def loaded(self, arm="K2", mode="sandbox"):
        argv = self.command(arm, mode)
        arg = argv[argv.index("--settings") + 1]
        with open(arg) as f:
            return json.load(f), arg

    def test_the_sandbox_is_required_and_cannot_be_left_by_the_agent(self):
        s, _ = self.loaded()
        sb = s["sandbox"]
        self.assertTrue(sb["enabled"])
        self.assertTrue(sb["failIfUnavailable"])
        self.assertFalse(sb["allowUnsandboxedCommands"])
        self.assertNotIn("excludedCommands", sb)

    def test_reads_are_denied_under_the_scratch_root_the_logs_and_the_repository_and_reopened_for_the_agents_own_directories(self):
        s, _ = self.loaded()
        fs = s["sandbox"]["filesystem"]
        r = lambda *p: "//" + os.path.join(self.root, *p).lstrip("/")
        self.assertEqual(fs["denyRead"][:3], [r("scratch"), r("logs"), r("repo")])
        for root in ("//Users", "//home", "//root"):  # and the user roots (#1519 review)
            self.assertIn(root, fs["denyRead"])
        for own in (r("scratch", "ta", "clone"), r("scratch", "ta", "home"), r("scratch", "origin.git"), r("bin")):
            self.assertIn(own, fs["allowRead"])
        self.assertNotIn(r("scratch", "tb"), fs["allowRead"])
        self.assertEqual(sorted(fs["allowWrite"]), sorted([r("scratch", "ta", "clone"), r("scratch", "ta", "home"), r("scratch", "ta", "tmp"),
                                                          r("scratch", "origin.git"), r("scratch", "klatch-home")]))
        self.assertNotIn(r("bin"), fs["allowWrite"])

    def test_the_file_tools_are_denied_the_other_agents_logs_and_repository_but_not_the_scratch_root(self):
        s, _ = self.loaded()
        deny = s["permissions"]["deny"]
        r = lambda *p: "//" + os.path.join(self.root, *p).lstrip("/")
        for p in (r("logs"), r("repo"), r("scratch", "tb")):
            for tool in ("Read", "Edit"):
                self.assertIn("%s(%s/**)" % (tool, p), deny)
        self.assertNotIn("Read(%s/**)" % r("scratch"), deny)  # a deny would also cover the agent's own clone inside it

    def test_the_arms_own_deny_rules_stay(self):
        self.assertIn("SendMessage", self.loaded("K2")[0]["permissions"]["deny"])
        s, _ = self.loaded("B1")
        self.assertEqual(s["crossSessionInbound"], "accept")
        self.assertFalse([d for d in s["permissions"]["deny"] if d in ("SendMessage", "ListAgents")])

    def test_the_settings_are_a_file_in_the_agents_directory_because_the_json_names_the_other_agents(self):
        s, arg = self.loaded()
        self.assertEqual(arg, os.path.join(self.root, "scratch", "ta", "settings.json"))
        argv = self.command()
        self.assertFalse([x for x in argv if "tb" in x.split(os.sep)])
        with open(os.path.join(self.root, "logs", "a.settings.json")) as f:  # the copy for the record
            self.assertEqual(json.load(f), s)

    def test_layout_mode_changes_nothing_in_the_settings_argument(self):
        argv = self.command("K2", "layout")
        arg = argv[argv.index("--settings") + 1]
        self.assertEqual(json.loads(arg), self.d.settings_for("K2"))
        self.assertNotIn("sandbox", json.loads(arg))

    def test_no_isolation_is_the_old_behaviour(self):
        argv, _ = self.d.command("a", "B0", "P", "/c", "/o")
        self.assertEqual(json.loads(argv[argv.index("--settings") + 1]), self.d.settings_for("B0"))

    def test_the_log_directory_is_hidden_from_real_agents_and_not_from_fake_ones_that_poll_it(self):
        self.assertFalse(self.d.lists_logs)
        self.assertTrue(drivers.FakeDriver("x").lists_logs)

    def test_every_arm_gets_the_same_sandbox_keys(self):
        keys = {arm: self.loaded(arm)[0]["sandbox"] for arm in ("B0", "B1", "K1", "K2")}
        self.assertEqual(len({json.dumps(v, sort_keys=True) for v in keys.values()}), 1)


if __name__ == "__main__":
    unittest.main()
