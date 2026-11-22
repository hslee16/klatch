"""Checks on clients/recipes/own-launchers.md (#126, #414): recipes for the launchers klatch itself ships or documents,
so a person without a desktop orchestrator still has something that was checked.

Rules the page must keep, each checked here:
- every `klatch` command in the page's Makefile also appears, same subcommand and same flags, in
  clients/recipes/launcher-demo.sh, which CI runs against the real binary;
- the Makefile parses and `make -n` of its targets prints the commands without running anything;
- every script and flag the page names (team, integrator, ask-supervisor) is in that script's own header or help text;
- the page names the third-party orchestrators as not run and points at #414;
- every repository file the page points at exists, and the page has no em dash.
Run from the repository root: `python3 -m unittest discover -s clients/recipes -p 'test_*.py'`.
"""
import os
import re
import shutil
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
PAGE = os.path.join(HERE, "own-launchers.md")
DEMO = os.path.join(HERE, "launcher-demo.sh")


def page():
    with open(PAGE, encoding="utf-8") as f:
        return f.read()


def makefile():
    m = re.search(r"```make\n(.*?)```", page(), re.S)
    assert m, "the page has a make block"
    return m.group(1)


def klatch_lines(text):
    """The (subcommand words and flags) of each `$(KLATCH) ...` or `klatch ...` command in text."""
    out = []
    for line in text.splitlines():
        m = re.search(r"(?:\$\(KLATCH\)|\"\$BIN\"|\bklatch)\s+(.*)", line)
        if m:
            words = [w for w in m.group(1).split() if re.match(r"^[a-z][a-z-]*$|^--[a-z-]+$", w)]
            out.append(words)
    return out


class OwnLaunchersDoc(unittest.TestCase):
    def test_the_page_exists(self):
        self.assertTrue(os.path.exists(PAGE))

    def test_each_make_klatch_command_is_one_the_ci_demo_runs(self):
        with open(DEMO, encoding="utf-8") as f:
            demo = f.read()
        cmds = klatch_lines(makefile())
        self.assertTrue(cmds, "the Makefile runs klatch")
        for words in cmds:
            plain = [w for w in words if not w.startswith("--")]
            sub = plain[:2] if plain[0] in ("run", "worker") else plain[:1]
            flags = [w for w in words if w.startswith("--")]
            self.assertIn(" ".join(sub), demo, sub)
            for flag in flags:
                self.assertIn(flag, demo, flag)

    @unittest.skipUnless(shutil.which("make"), "make is not installed")
    def test_make_dry_run_prints_and_runs_nothing(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "Makefile")
            with open(path, "w", encoding="utf-8") as f:
                f.write(makefile())
            marker = os.path.join(d, "ran")
            env = dict(os.environ, KLATCH="touch " + marker + " #")
            for target in ("issue", "work-1", "done-1", "end"):
                r = subprocess.run(["make", "-n", "-f", path, target], cwd=d, env=env,
                                   capture_output=True, text=True)
                self.assertEqual(r.returncode, 0, target + ": " + r.stderr)
                self.assertTrue(r.stdout.strip(), target)
            self.assertFalse(os.path.exists(marker), "make -n ran a command")

    def test_the_scripts_and_flags_named_exist_in_their_headers(self):
        text = page()
        for script, flags in (("team", ["start", "status", "stop", "smoke"]),
                              ("integrator", ["--dry-run", "--once"]),
                              ("ask-supervisor", ["--dry-run", "--once"])):
            path = os.path.join(ROOT, "proto", "scripts", script)
            self.assertTrue(os.path.exists(path), script)
            with open(path, encoding="utf-8") as f:
                header = "".join(f.readlines()[:60])
            self.assertIn("proto/scripts/" + script, text)
            for flag in flags:
                self.assertIn(flag, header, script + " " + flag)
                self.assertIn(flag, text, script + " " + flag)

    def test_third_party_orchestrators_are_still_not_run_and_point_at_414(self):
        text = page()
        self.assertIn("#414", text)
        self.assertRegex(text, r"(?i)not run")
        for tool in ("Claude Squad", "Conductor", "Orchestrator.inc", "Agent Swarm"):
            self.assertIn(tool, text)

    def test_a_launcher_never_runs_inside_an_agent_session_is_stated(self):
        self.assertRegex(page(), r"(?i)outside (any|an) (coding-)?agent session|your own terminal")

    def test_the_files_it_points_at_exist(self):
        for target in re.findall(r"\]\(([^)#]+)(?:#[^)]*)?\)", page()):
            if target.startswith("http"):
                continue
            self.assertTrue(os.path.exists(os.path.normpath(os.path.join(HERE, target))), target)

    def test_no_em_dash(self):
        self.assertNotIn("—", page())


if __name__ == "__main__":
    unittest.main()
