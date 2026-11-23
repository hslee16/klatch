"""Command construction for the real driver. Nothing here starts claude: it only checks the argv and the environment the
driver would use (SPEC 2.1, 8). Real runs are the human's call after the budget approval."""
import unittest

from helpers import BENCH  # noqa: F401
import drivers


def make(arm="K2", **kw):
    args = dict(claude_bin="/usr/local/bin/claude", model="claude-haiku-4-5-20251001", max_turns=40,
                per_agent_usd=0.5, klatch_bin="/opt/klatch/klatch")
    args.update(kw)
    return drivers.ClaudeDriver(**args), arm


class Command(unittest.TestCase):
    def argv(self, arm="K2", **kw):
        d, arm = make(arm, **kw)
        argv, _ = d.command(name="a", arm=arm, prompt="PROMPT TEXT", cwd="/tmp/x/clone", out_dir="/tmp/x/out")
        return argv

    def test_headless_stream_json_with_the_pinned_model(self):
        a = self.argv()
        self.assertEqual(a[0], "/usr/local/bin/claude")
        self.assertEqual(a[a.index("-p") + 1], "PROMPT TEXT")
        self.assertEqual(a[a.index("--model") + 1], "claude-haiku-4-5-20251001")
        self.assertEqual(a[a.index("--output-format") + 1], "stream-json")
        for flag in ("--verbose", "--include-hook-events", "--no-session-persistence", "--strict-mcp-config"):
            self.assertIn(flag, a)

    def test_settings_come_from_the_project_only(self):
        a = self.argv()
        self.assertEqual(a[a.index("--setting-sources") + 1], "local")

    def test_caps_are_passed(self):
        a = self.argv()
        self.assertEqual(a[a.index("--max-budget-usd") + 1], "0.50")
        self.assertEqual(a[a.index("--max-turns") + 1], "40")

    def test_never_skips_permissions(self):
        self.assertNotIn("--dangerously-skip-permissions", self.argv())
        self.assertEqual(self.argv()[self.argv().index("--permission-mode") + 1], "acceptEdits")

    def allowed(self, arm):
        a = self.argv(arm)
        i = a.index("--allowedTools") + 1
        out = []
        while i < len(a) and not a[i].startswith("--"):
            out.append(a[i])
            i += 1
        return out

    def test_klatch_arms_may_run_the_klatch_binary_by_absolute_path_and_b_arms_may_not(self):
        self.assertIn("Bash(/opt/klatch/klatch:*)", self.allowed("K1"))
        self.assertIn("Bash(/opt/klatch/klatch:*)", self.allowed("K2"))
        self.assertNotIn("Bash(/opt/klatch/klatch:*)", self.allowed("B0"))
        self.assertNotIn("Bash(/opt/klatch/klatch:*)", self.allowed("B1"))

    def test_b1_gets_the_native_messaging_tool_and_nobody_else_does(self):
        self.assertIn("SendMessage", self.allowed("B1"))
        for arm in ("B0", "K1", "K2"):
            self.assertNotIn("SendMessage", self.allowed(arm))

    def test_every_arm_gets_the_same_file_and_git_tools(self):
        common = {"Read", "Edit", "Write", "Glob", "Grep", "Bash(git:*)", "Bash(python3:*)"}
        for arm in ("B0", "B1", "K1", "K2"):
            self.assertTrue(common <= set(self.allowed(arm)), arm)

    def test_the_prompt_is_the_only_free_text_in_the_command(self):
        # the cwd, the klatch path and the budget are flags, never pasted into the prompt by the driver
        self.assertEqual(self.argv().count("PROMPT TEXT"), 1)


class Environment(unittest.TestCase):
    BASE = {"PATH": "/usr/bin", "LANG": "C", "ANTHROPIC_API_KEY": "sk-sentinel", "AWS_SECRET_ACCESS_KEY": "x",
            "GITHUB_TOKEN": "y", "HOME": "/Users/real"}

    def test_only_the_model_credential_passes_and_only_to_the_claude_driver(self):
        d, _ = make()
        env = d.agent_env(self.BASE, scratch="/tmp/s")
        self.assertEqual(env["ANTHROPIC_API_KEY"], "sk-sentinel")
        for k in ("AWS_SECRET_ACCESS_KEY", "GITHUB_TOKEN"):
            self.assertNotIn(k, env)
        self.assertEqual(env["CLAUDE_CONFIG_DIR"], "/tmp/s/claude-config")

    def test_fake_driver_gets_no_credentials(self):
        env = drivers.FakeDriver(script_path="x").agent_env(self.BASE, scratch="/tmp/s")
        self.assertNotIn("ANTHROPIC_API_KEY", env)
        self.assertNotIn("CLAUDE_CONFIG_DIR", env)

    def test_no_credential_means_none_is_set(self):
        d, _ = make()
        env = d.agent_env({"PATH": "/usr/bin"}, scratch="/tmp/s")
        self.assertNotIn("ANTHROPIC_API_KEY", env)


class Version(unittest.TestCase):
    def test_parse_claude_version(self):
        self.assertEqual(drivers.parse_claude_version("2.1.5 (Claude Code)\n"), "2.1.5")
        self.assertIsNone(drivers.parse_claude_version("what"))


if __name__ == "__main__":
    unittest.main()
