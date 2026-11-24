"""The real-session probe of the isolation settings (isolation_probe.py): what it would run, without running claude."""
import json
import os
import shutil
import stat
import tempfile
import unittest

from helpers import BENCH  # noqa: F401
import isolation_probe

REPO = os.path.dirname(os.path.dirname(BENCH))


class Session(unittest.TestCase):
    def setUp(self):
        self.tmp = os.path.realpath(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.tmp, ignore_errors=True))
        self.src_dir = os.path.join(REPO, "proto", "target", "debug")  # where a developer's binary lives: inside the repository
        self.src = os.path.join(self.tmp, "klatch-src")  # a stand-in file (the test never runs it)
        with open(self.src, "w") as f:
            f.write("#!/bin/sh\n")
        os.chmod(self.src, 0o755)
        self.root = os.path.join(self.tmp, "probe")
        os.mkdir(self.root)
        isolation_probe.build(self.root)
        self.s = isolation_probe.make_session(self.root, isolation_probe.Opts("claude", "m", self.src, "lobby"), self.tmp)
        self.addCleanup(lambda: shutil.rmtree(self.s["staged_dir"], ignore_errors=True))
        self.argv = self.s["argv"]

    def allowed(self):
        i = self.argv.index("--allowedTools") + 1
        out = []
        while i < len(self.argv) and not self.argv[i].startswith("--"):
            out.append(self.argv[i])
            i += 1
        return out

    def test_the_session_is_arm_k1_so_the_klatch_tool_is_allowed_and_the_binary_is_the_staged_copy(self):
        staged = os.path.join(self.s["staged_dir"], "klatch-src")
        self.assertIn("Bash(%s:*)" % staged, self.allowed())
        self.assertTrue(os.path.isfile(staged))
        self.assertRegex(self.s["staged_dir"], r"/cb-bin-[^/]+$")
        self.assertTrue(self.s["staged_dir"].startswith(self.tmp))
        self.assertFalse(self.s["staged_dir"].startswith(REPO))
        self.assertNotIn(self.src, " ".join(self.argv))
        self.assertIn("SendMessage", json.dumps(json.load(open(self.s["settings"]))))  # K1 settings (messaging tools denied)

    def test_the_prompt_uses_the_k1_commands_and_the_arms_room_with_the_staged_path(self):
        prompt = self.argv[self.argv.index("-p") + 1]
        staged = os.path.join(self.s["staged_dir"], "klatch-src")
        for cmd in ("%s --version" % staged, "%s post lobby" % staged, "%s read lobby" % staged, "%s claim lobby" % staged,
                    "%s release lobby" % staged):
            self.assertIn(cmd, prompt)
        self.assertNotIn(self.src, prompt)

    def test_it_has_room_for_every_item_plus_four_turns_and_a_session_cap_of_015(self):
        items = len(self.s["tries"]) + 1  # and the Read tool item
        self.assertEqual(self.argv[self.argv.index("--max-turns") + 1], str(items + 4))
        self.assertEqual(self.argv[self.argv.index("--max-budget-usd") + 1], "0.15")
        self.assertIn("0.08 to 0.10 USD", isolation_probe.__doc__)

    def test_the_environment_is_the_one_the_harness_gives_a_k_agent(self):
        env = self.s["env"]
        for k in ("KLATCH_HOME", "KLATCH_KEY_DIR", "KLATCH_NAME", "KLATCH_BIN", "HOME", "TMPDIR", "CLAUDE_CONFIG_DIR", "PATH"):
            self.assertIn(k, env, k)
        self.assertEqual(env["KLATCH_BIN"], os.path.join(self.s["staged_dir"], "klatch-src"))
        for k in ("KLATCH_HOME", "KLATCH_KEY_DIR"):
            self.assertTrue(env[k].startswith(self.root))
            self.assertEqual(stat.S_IMODE(os.stat(env[k]).st_mode), 0o700)
        self.assertTrue(env["HOME"].startswith(os.path.join(self.root, "agent-a")))
        self.assertTrue(env["TMPDIR"].startswith(os.path.join(self.root, "agent-a")))

    def test_the_settings_let_the_store_be_opened_written_and_read_and_the_binary_read(self):
        fs = json.load(open(self.s["settings"]))["sandbox"]["filesystem"]
        r = lambda p: "//" + os.path.realpath(p).lstrip("/")
        for k in ("KLATCH_HOME", "KLATCH_KEY_DIR"):
            self.assertIn(r(self.s["env"][k]), fs["allowWrite"], k)
            self.assertIn(r(self.s["env"][k]), fs["allowRead"], k)
        self.assertIn(r(self.s["staged_dir"]), fs["allowRead"])
        self.assertNotIn(r(self.s["staged_dir"]), fs["allowWrite"])

    def test_the_klatch_items_come_with_the_escape_items_and_each_klatch_one_names_what_it_exercises(self):
        labels = [t[0] for t in self.s["tries"]]
        self.assertEqual(len([t for t in self.s["tries"] if len(t) > 3 and t[3] == "klatch"]), 5)
        for word in ("--version", "post", "read", "claim", "release"):
            self.assertTrue([l for l in labels if word in l], word)
        self.assertEqual(len([t for t in self.s["tries"] if len(t) < 4 or t[3] == "escape"]), 6)  # the earlier six escapes (plus the Read item)

    def test_without_a_klatch_binary_the_session_is_the_old_one(self):
        root = os.path.join(self.tmp, "probe2")
        os.mkdir(root)
        isolation_probe.build(root)
        s = isolation_probe.make_session(root, isolation_probe.Opts("claude", "m", None, "lobby"), self.tmp)
        self.assertEqual(len(s["tries"]), 6)
        self.assertIsNone(s["staged_dir"])


class Table(unittest.TestCase):
    def test_classify_tells_a_sandbox_refusal_from_a_missing_allow_rule_from_a_missing_file(self):
        c = isolation_probe.classify
        self.assertEqual(c("python3: can't open file: [Errno 1] Operation not permitted: '/x'"), "sandbox refusal")
        self.assertEqual(c("PermissionError: [Errno 13] Permission denied: '/x'"), "sandbox refusal")
        self.assertEqual(c("This command requires approval"), "missing allow rule")
        self.assertEqual(c("cat: /x: No such file or directory"), "missing file")
        self.assertEqual(c("klatch 0.3.0"), "ok")
        self.assertEqual(c(""), "ok")

    def test_the_first_error_line_is_the_first_nonempty_line_cut(self):
        self.assertEqual(isolation_probe.first_line("\n\n  Operation not permitted\nmore"), "Operation not permitted")
        self.assertEqual(len(isolation_probe.first_line("x" * 500)), 120)

    def test_every_row_shows_the_class_and_the_first_line_and_a_klatch_item_is_ok_or_failed(self):
        tries = [("control", "cmd", "OWN-SECRET"), ("klatch post", "cmd", "", "klatch"), ("klatch read", "cmd", "sandbox check", "klatch"),
                 ("klatch claim", "cmd", "", "klatch")]
        results = ["OWN-SECRET", "posted #1", "Operation not permitted\n(os error 1)", "This command requires approval"]
        rows = []
        isolation_probe.print_table(tries, results, rows.append)
        text = "\n".join(rows)
        self.assertIn("[ok] OWN-SECRET", rows[0])
        self.assertIn("OK", rows[1])
        self.assertIn("FAILED", rows[2])
        self.assertIn("[sandbox refusal] Operation not permitted", rows[2])
        self.assertIn("[missing allow rule] This command requires approval", rows[3])
        self.assertNotIn("(os error 1)", text)


if __name__ == "__main__":
    unittest.main()
