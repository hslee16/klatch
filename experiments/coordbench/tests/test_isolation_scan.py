"""The post-run scan of tool calls (SPEC 11, #1519): fake streams with calls inside and outside the agent's directories."""
import json
import os
import tempfile
import unittest

from helpers import BENCH  # noqa: F401
import isolation


def call(name, **inp):
    return json.dumps({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "t", "name": name, "input": inp}]}})


def bash(cmd):
    return call("Bash", command=cmd)


class Scan(unittest.TestCase):
    def setUp(self):
        self.root = os.path.realpath(tempfile.mkdtemp())
        self.addCleanup(lambda: __import__("shutil").rmtree(self.root, ignore_errors=True))
        j = lambda *p: os.path.join(self.root, *p)
        self.clone, self.home, self.tmp = j("tok-a", "clone"), j("tok-a", "home"), j("tok-a", "tmp")
        self.origin, self.bin = j("origin.git"), j("bin")
        self.other = j("tok-b", "clone")
        self.logs = j("logs", "run-1-tok")
        for d in (self.clone, self.home, self.tmp, self.origin, self.bin, self.other, self.logs):
            os.makedirs(d)
        self.policy = isolation.Policy("a", self.clone, self.home, self.tmp, allow=[self.origin, self.bin],
                                       guard=[self.root, j("logs"), j("fixtures"), j("checks")])

    def scan(self, *lines):
        return isolation.scan_lines(list(lines), self.policy)

    def test_calls_inside_the_clone_are_clean(self):
        n, v = self.scan(call("Read", file_path=self.clone + "/a.py"), call("Edit", file_path="src/x.py", old_string="../x"),
                         call("Write", file_path="b.py", content="open('../c/x')"), call("Glob", pattern="**/*.py"),
                         call("Grep", pattern="..", path="."), bash("git status && python3 -m unittest discover -s tests"),
                         bash("git push origin HEAD:main"), bash("%s/klatch send hi" % self.bin),
                         bash("git -C %s log --oneline" % self.origin), bash("echo $HOME/x $TMPDIR/y; cd src && cat ../a.py"))
        self.assertEqual((n, v), (10, []))

    def test_a_relative_path_into_a_sibling_clone_is_a_violation(self):
        n, v = self.scan(bash("cat ../b/SPEC-ROUTING.md"))
        self.assertEqual(len(v), 1)
        self.assertEqual((v[0]["agent"], v[0]["tool"], v[0]["call"]), ("a", "Bash", 1))
        self.assertIn("../b/SPEC-ROUTING.md", v[0]["text"])

    def test_a_python_one_liner_that_opens_a_sibling_is_a_violation(self):
        _, v = self.scan(bash("python3 -c \"open('../c/x').read()\""))
        self.assertEqual(len(v), 1)

    def test_an_absolute_path_to_another_agents_clone_is_a_violation_in_every_tool(self):
        p = self.other + "/SPEC.md"
        _, v = self.scan(call("Read", file_path=p), call("Glob", pattern=self.other + "/*.md"), call("Grep", pattern="x", path=self.other),
                         call("Edit", file_path=p, old_string="a", new_string="b"), call("Write", file_path=p, content="x"),
                         bash("git -C %s log" % self.other), bash("python3 -c \"print(open('%s').read())\"" % p))
        self.assertEqual([x["call"] for x in v], [1, 2, 3, 4, 5, 6, 7])

    def test_the_log_directory_and_the_checker_and_fixture_directories_are_violations(self):
        for p in (self.logs + "/a.prompt.md", os.path.join(self.root, "logs"), os.path.join(self.root, "checks", "x", "check.py"),
                  os.path.join(self.root, "fixtures", "x", "reference", "y.py")):
            _, v = self.scan(call("Read", file_path=p))
            self.assertEqual(len(v), 1, p)

    def test_parent_directories_and_cd_out_of_the_clone(self):
        for cmd in ("ls ..", "cd .. && ls", "cd ../.. && cat tok-b/clone/x", "git -C ../ status", "python3 -c \"import os;print(os.listdir('..'))\""):
            _, v = self.scan(bash(cmd))
            self.assertTrue(v, cmd)

    def test_cd_is_followed_across_calls_so_dot_dot_in_a_subdirectory_is_fine(self):
        os.makedirs(os.path.join(self.clone, "src"))
        n, v = self.scan(bash("cd src"), bash("cat ../a.py"), bash("cd .."), bash("ls"))
        self.assertEqual(v, [])
        _, v = self.scan(bash("cd src"), bash("cat ../../x"))
        self.assertEqual(len(v), 1)

    def test_a_symlink_in_the_clone_that_points_out_is_followed(self):
        os.symlink(self.other, os.path.join(self.clone, "link"))
        _, v = self.scan(bash("cat ./link/SPEC.md"), call("Read", file_path=self.clone + "/link/SPEC.md"))
        self.assertEqual(len(v), 2)

    def test_the_users_home_and_the_live_stores_are_violations(self):
        _, v = self.scan(bash("ls ~/../x"), bash("cat /Users/someone/.klatch/store"), bash("cat /home/x/.claude/settings.json"))
        self.assertEqual(len(v), 3)

    def test_listing_processes_is_a_violation_but_a_word_ps_is_not(self):
        _, v = self.scan(bash("ps aux"), bash("python3 -c \"import subprocess;subprocess.run(['pgrep','-a','claude'])\""),
                         bash("cat /proc/1/environ"))
        self.assertEqual(len(v), 3)
        n, v = self.scan(bash("git commit -m 'adds ps support and the tps module'"), bash("python3 -c \"print('maps')\""))
        self.assertEqual(v, [])

    def test_a_heredoc_that_only_writes_a_file_is_not_scanned_but_one_fed_to_python_is(self):
        _, v = self.scan(bash("cat > mod.py <<'EOF'\nimport os\nos.listdir('../c')\nEOF\ngit add mod.py"))
        self.assertEqual(v, [])
        _, v = self.scan(bash("python3 - <<'EOF'\nimport os\nprint(open('../c/x').read())\nEOF"))
        self.assertEqual(len(v), 1)

    def test_urls_and_prose_are_not_paths(self):
        n, v = self.scan(bash("git clone https://github.com/x/y.git; echo 'a/b and/or c'; python3 -c \"print(3/4)\""))
        self.assertEqual(v, [])

    def test_a_stream_with_noise_lines_is_read(self):
        n, v = isolation.scan_lines(["", "not json", json.dumps({"type": "result"}), bash("cat ../b/x")], self.policy)
        self.assertEqual((n, len(v)), (1, 1))

    def test_describe_names_the_first_violations(self):
        _, v = self.scan(bash("cat ../b/x"))
        self.assertIn("isolation violation", isolation.describe(v))
        self.assertIn("../b/x", isolation.describe(v))


if __name__ == "__main__":
    unittest.main()
