"""Review fixes of #1538 (#1519): restart directory, poll and timestamps, root-anchored searches, log layout, signals,
which directories the harness may chmod, the Claude Code version, the scan's false positives and the record field."""
import contextlib
import io
import json
import os
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

from helpers import BENCH  # noqa: F401
import drivers
import isolation
import run
import runpoll
import schema
from test_isolation_run import Base, EscapeDriver
from test_isolation_scan import bash, call
from test_roles import RoleBase


def settings_of(**kw):
    root = kw.pop("root")
    j = lambda *p: os.path.join(root, *p)
    iso = {"mode": "sandbox", "root": j("scratch"), "own": [j("scratch", "agents", "ta", "clone"), j("scratch", "agents", "ta", "home")],
           "write": [j("scratch", "origin.git")], "read": [j("bin")], "guard": [j("scratch"), j("logs"), j("repo")],
           "others": [], "settings_dir": j("scratch")}
    iso.update(kw)
    return isolation.sandbox_settings(iso)


class RoleRestart(RoleBase):
    """Item 1: a role member writes into the restart directory from sandboxed Bash."""

    def capture(self, **patches):
        seen = []
        orig = drivers.FakeDriver.command

        def spy(me, name, arm, prompt, cwd, out_dir, **kw):
            seen.append((name, kw.get("role"), kw.get("isolation")))
            return orig(me, name, arm, prompt, cwd, out_dir, **kw)
        with mock.patch.object(drivers.FakeDriver, "command", spy):
            self.go(self.argv("B1-roles", 1, **patches))
        return seen

    def test_the_restart_directory_is_writable_for_role_members_and_not_for_workers(self):
        seen = self.capture()
        roles = [iso for n, role, iso in seen if role]
        workers = [iso for n, role, iso in seen if not role and iso]
        self.assertTrue(roles and workers)
        for iso in roles:
            self.check_role(iso)
        for iso in workers:
            self.assertFalse([p for p in iso["write"] + iso["read"] if p.endswith(os.sep + "restart")])

    def check_role(self, iso):
        restart = [p for p in iso["write"] if p.endswith(os.sep + "restart")]
        self.assertEqual(len(restart), 1, iso["write"])
        fs = isolation.sandbox_settings(iso)["sandbox"]["filesystem"]
        self.assertIn("//" + os.path.realpath(restart[0]).lstrip("/"), fs["allowWrite"])
        self.assertTrue(any(p.endswith(os.sep + "bin") for p in iso["read"]))

    def test_a_role_launch_without_the_copied_restart_script_is_refused_before_any_agent_starts(self):
        with mock.patch("runexec.shutil.copy", lambda src, dst: dst):
            self.go(self.argv("B1-roles", 1))
        recs = self.records()
        self.assertEqual(len(recs), 1)
        self.assertEqual(recs[0]["outcome"], "error")
        self.assertFalse(recs[0]["valid"])
        self.assertIn("restart script", recs[0]["notes"])


class FakeRun:
    """What runpoll.PollSupport needs of a run."""

    def __init__(self, tmp):
        self.log_dir, self.t0 = tmp, 0.0
        self.procs, self.offs, self.partial, self.stamps, self.states = {"a": None}, {"a": 0}, {"a": b""}, {"a": []}, {}
        self.fed = []
        self.states["a"] = mock.Mock(feed=lambda line, now: self.fed.append((line, now)))
        self.a = mock.Mock(poll_s=0.0)
        self.first_pass, self.last_poll = None, 0.0
        self.is_shared = False
        self.checks_calls = 0

    def log_path(self, key, kind):
        return os.path.join(self.log_dir, "%s.%s" % (key, kind))

    def all_pass(self, checks):
        return True


class Pump(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(lambda: shutil.rmtree(self.tmp, ignore_errors=True))

    def make(self):
        cls = type("R", (FakeRun, runpoll.PollSupport), {})
        return cls(self.tmp)

    def test_every_line_of_a_batch_gets_its_own_clock_reading(self):
        r = self.make()
        with open(r.log_path("a", "stream"), "wb") as f:
            f.write(b"one\ntwo\nthree\npart")
        with mock.patch("runpoll.time.monotonic", side_effect=[i * 0.001 for i in range(1, 50)]):
            r.pump()
        stamps = [now for _, now in r.fed]
        self.assertEqual([l for l, _ in r.fed], ["one", "two", "three"])
        self.assertEqual(len(set(stamps)), 3)
        self.assertEqual(stamps, sorted(stamps))
        self.assertEqual(r.stamps["a"], stamps)

    def test_the_hidden_checker_runs_in_a_thread_so_the_watch_loop_is_not_held_up(self):
        r = self.make()
        release = threading.Event()

        def slow(tag):
            release.wait(5)
            return [{"id": "u", "pass": True, "unit": True}], "", None
        r.checks_now = slow
        t = time.monotonic()
        r.poll_checker()
        self.assertLess(time.monotonic() - t, 0.5)  # returned while the checker is still running
        self.assertIsNone(r.first_pass)
        r.poll_checker()  # still running: no second checker is started
        release.set()
        r.join_poll()
        r.poll_checker()
        self.assertIsNotNone(r.first_pass)


class Roots(unittest.TestCase):
    """Item 3: the real home, /Users and /home are denied and only the run's own directories are re-opened."""

    def setUp(self):
        self.root = os.path.realpath(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))

    def test_denyread_names_the_user_roots_and_the_real_home_and_allowread_the_run_and_the_tools(self):
        s = settings_of(root=self.root, read=[os.path.join(self.root, "bin"), "/opt/homebrew/Cellar/python"])
        fs = s["sandbox"]["filesystem"]
        for p in ("//Users", "//home", "//root", "//" + os.path.realpath(os.path.expanduser("~")).lstrip("/")):
            self.assertIn(p, fs["denyRead"])
        self.assertIn("//opt/homebrew/Cellar/python", fs["allowRead"])

    def test_the_file_tools_are_denied_the_user_roots_with_absolute_paths_never_tilde(self):
        deny = settings_of(root=self.root)["permissions"]["deny"]
        for p in ("//Users", "//home", "//root"):
            for tool in ("Read", "Edit"):
                self.assertIn("%s(%s/**)" % (tool, p), deny)
        self.assertFalse([d for d in deny if "~" in d])

    def test_a_user_root_that_contains_the_agents_own_directory_is_left_out_of_the_file_tool_rules(self):
        s = settings_of(root=self.root, own=["/Users/x/run/clone", "/Users/x/run/home"])
        deny = s["permissions"]["deny"]
        self.assertNotIn("Read(//Users/**)", deny)
        self.assertIn("//Users", s["sandbox"]["filesystem"]["denyRead"])  # the sandbox can re-allow, a file-tool deny cannot

    def test_the_tool_roots_are_the_install_directories_of_python3_and_git(self):
        roots = isolation.tool_roots()
        self.assertTrue(roots)
        for r in roots:
            self.assertTrue(os.path.isdir(r))

    def test_no_local_binding_and_no_network_keys_at_all(self):
        self.assertNotIn("network", settings_of(root=self.root)["sandbox"])


class RootSearch(unittest.TestCase):
    def setUp(self):
        self.root = os.path.realpath(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))
        j = lambda *p: os.path.join(self.root, *p)
        os.makedirs(j("run", "agents", "ta", "clone"))
        self.policy = isolation.Policy("a", (j("run", "agents", "ta", "clone"), j("run", "agents", "ta", "home"), j("run", "agents", "ta", "tmp")),
                                       allow=[j("bin"), isolation.Exact(j("run", "agents", "ta"))], guard=[j("run")])

    def scan(self, *lines):
        return isolation.scan_lines(list(lines), self.policy)[1]

    def test_searches_rooted_at_the_filesystem_root_the_user_roots_or_a_parent_of_the_run_are_violations(self):
        for cmd in ("find / -name 'SPEC*'", "rg -l SECRET /", "grep -r token /Users", "ls /home", "find %s -name x" % self.root,
                    "python3 -c \"import os;print(os.listdir('/'))\""):
            self.assertTrue(self.scan(bash(cmd)), cmd)

    def test_grep_and_glob_rooted_at_a_root_or_with_a_wildcard_in_the_first_segments_are_violations(self):
        for line in (call("Grep", pattern="x", path="/"), call("Grep", pattern="x", path="/Users"), call("Glob", pattern="/U*/*/klatch-*/x"),
                     call("Glob", pattern="/*/*/klatch/**"), call("Glob", pattern="/", path="/")):
            self.assertTrue(self.scan(line), line)

    def test_ordinary_absolute_paths_are_not(self):
        self.assertEqual(self.scan(bash("ls /usr/bin | head -3; cat /etc/hosts; python3 /usr/lib/python3/x.py")), [])
        self.assertEqual(self.scan(call("Glob", pattern="**/*.py"), call("Grep", pattern="x", path=".")), [])

    def test_the_agents_own_directory_above_its_clone_is_its_own(self):
        self.assertEqual(self.scan(bash("ls ..")), [])
        self.assertTrue(self.scan(bash("ls ../..")))
        self.assertTrue(self.scan(bash("cat ../b/SPEC.md")))

    def test_a_quoted_ps_in_a_grep_is_not_process_listing_but_a_list_argument_is(self):
        self.assertEqual(self.scan(bash("grep -rn 'ps' .")), [])
        self.assertEqual(self.scan(bash("grep -rn \"ps\" . && git log --grep='ps'")), [])
        self.assertTrue(self.scan(bash("python3 -c \"import subprocess;subprocess.run(['ps','-ww'])\"")))
        self.assertTrue(self.scan(bash("python3 -c \"import os;os.system('ps aux')\"")))


class LogLayout(Base):
    def setUp(self):
        super().setUp()
        self.driver = EscapeDriver()
        self.seen = {}
        orig = EscapeDriver.command

        def spy(me, name, arm, prompt, cwd, out_dir, **kw):
            self.seen[name] = sorted(os.path.basename(p) for p in kw["isolation"].get("log_files", []))
            self.during = kw["isolation"]
            return orig(me, name, arm, prompt, cwd, out_dir, **kw)
        with mock.patch.object(EscapeDriver, "command", spy):
            self.run_with(self.driver)
        self.rec = self.record()
        self.logdir = self.rec["artifacts"]["streams"]

    def test_during_the_run_the_log_files_have_random_names_and_afterwards_the_public_ones(self):
        self.assertEqual(sorted(os.listdir(self.logdir))[:1], ["a.err"])
        for n in ("a.prompt.md", "a.stream.jsonl", "b.stream.jsonl", "check.json"):
            self.assertTrue(os.path.exists(os.path.join(self.logdir, n)), n)
        self.assertTrue(self.during["log_files"])
        for p in self.during["log_files"]:
            self.assertNotRegex(os.path.basename(p), r"^[ab]\.(stream|prompt|err)")

    def test_the_log_directory_is_staged_under_a_random_name_inside_the_users_logs_dir_and_published_by_run_id(self):
        self.assertEqual(os.path.basename(self.logdir), "t1-trivial-b0-r01")
        self.assertEqual([n for n in os.listdir(self.logs) if not n.startswith(".")], ["t1-trivial-b0-r01"])
        self.assertEqual(stat.S_IMODE(os.stat(self.logdir).st_mode), 0o700)

    def test_the_agents_cannot_read_the_logs_by_the_old_predictable_names(self):
        with open(os.path.join(self.logdir, "a.escape.json")) as f:
            report = {r["label"]: r for r in json.load(f)}
        self.assertEqual(report["read the log a.prompt.md by its public name"]["outcome"], "blocked")
        self.assertEqual(report["read the log by its leaked random name"]["outcome"], "read")


class Dirs(Base):
    def test_the_users_logs_dir_keeps_its_mode_and_is_never_hidden(self):
        os.mkdir(self.logs, 0o755)
        os.chmod(self.logs, 0o755)
        self.run_with(EscapeDriver())
        self.assertEqual(stat.S_IMODE(os.stat(self.logs).st_mode), 0o755)

    def test_a_second_live_run_under_the_same_logs_root_keeps_this_runs_logs_unlistable(self):
        os.mkdir(self.logs)
        with open(os.path.join(self.logs, ".live.other"), "w") as f:
            f.write(str(os.getpid()))  # a live owner
        self.run_with(EscapeDriver())
        rec = self.record()
        self.assertEqual(stat.S_IMODE(os.stat(rec["artifacts"]["streams"]).st_mode), 0o300)
        self.assertIn("another run is live", rec["notes"])

    def test_a_stale_marker_of_a_dead_process_does_not_count(self):
        os.mkdir(self.logs)
        dead = subprocess.Popen([sys.executable, "-c", "pass"])
        dead.wait()
        with open(os.path.join(self.logs, ".live.old"), "w") as f:
            f.write(str(dead.pid))
        self.run_with(EscapeDriver())
        self.assertEqual(stat.S_IMODE(os.stat(self.record()["artifacts"]["streams"]).st_mode), 0o700)

    def test_unsafe_roots_are_refused(self):
        home = os.environ["HOME"]
        self.assertIn("home directory", isolation.validate_dir(home, "--logs-dir"))
        loose = os.path.join(self.tmp, "loose")
        os.mkdir(loose)
        os.chmod(loose, 0o775)
        self.assertIn("writable", isolation.validate_dir(loose, "--tmp-root"))
        ok = os.path.join(self.tmp, "ok")
        os.mkdir(ok, 0o700)
        self.assertIsNone(isolation.validate_dir(ok, "--tmp-root"))
        self.assertIsNone(isolation.validate_dir(os.path.join(self.tmp, "not-yet"), "--logs-dir"))

    def test_main_refuses_with_exit_2_and_writes_nothing(self):
        for flag in ("logs_dir", "tmp_root"):
            argv = self.argv("B0", **{flag: self.home})
            out = io.StringIO()
            with contextlib.redirect_stderr(out):
                code = run.main(argv)
            self.assertEqual(code, 2, flag)
            self.assertFalse(os.path.exists(self.results), flag)
            self.assertIn("home directory", out.getvalue())

    def test_a_chmod_that_fails_is_a_clear_refusal(self):
        with mock.patch("isolation.os.chmod", side_effect=PermissionError(1, "Operation not permitted")):
            with self.assertRaises(isolation.IsolationError) as cm:
                isolation.hide(self.tmp)
        self.assertIn("cannot hide", str(cm.exception))


class Signals(Base):
    def test_restore_all_restores_every_directory_hidden_by_this_process(self):
        d1, d2 = os.path.join(self.tmp, "h1"), os.path.join(self.tmp, "h2")
        os.mkdir(d1)
        os.mkdir(d2)
        isolation.hide(d1)
        isolation.hide(d2)
        self.assertEqual(stat.S_IMODE(os.stat(d1).st_mode), 0o300)
        isolation.restore_all()
        for d in (d1, d2):
            self.assertEqual(stat.S_IMODE(os.stat(d).st_mode), 0o700)

    def test_sigterm_leaves_nothing_unremovable_and_a_sweep_clears_a_crash(self):
        script = os.path.join(self.tmp, "hang.json")
        with open(script, "w") as f:
            json.dump({"agents": {"a": [{"op": "hang"}], "b": [{"op": "hang"}]}}, f)
        argv = [sys.executable, os.path.join(BENCH, "run.py")] + self.argv("B0", fake_script=script)
        p = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=dict(os.environ))
        deadline = time.time() + 30
        while time.time() < deadline and not self.agents_started():
            time.sleep(0.1)
        time.sleep(0.5)  # the agents are running, the directories are hidden
        p.send_signal(signal.SIGTERM)
        p.wait(timeout=30)
        for d, dirs, _ in os.walk(self.tmp):
            for x in dirs:
                self.assertTrue(os.access(os.path.join(d, x), os.R_OK | os.X_OK), os.path.join(d, x))
        shutil.rmtree(self.scratch)  # a plain rm -rf works
        self.assertNotEqual(p.returncode, 0)

    def agents_started(self):
        """The hidden directories exist: the run is past its setup."""
        runs = [os.path.join(self.scratch, n) for n in os.listdir(self.scratch)] if os.path.isdir(self.scratch) else []
        return any(os.path.isdir(os.path.join(r, "agents")) and stat.S_IMODE(os.stat(os.path.join(r, "agents")).st_mode) == 0o300 for r in runs)

    def test_the_startup_sweep_restores_and_removes_stale_scratch_of_dead_runs_and_leaves_live_ones(self):
        dead = subprocess.Popen([sys.executable, "-c", "pass"])
        dead.wait()
        mk = lambda name, pid: self.make_hidden(name, pid)
        stale, live = mk("cb-old-abc", dead.pid), mk("cb-live-abc", os.getpid())
        foreign = os.path.join(self.scratch, "cb-nomarker")
        os.mkdir(foreign)
        isolation.sweep_stale(self.scratch)
        self.assertFalse(os.path.exists(stale))
        self.assertTrue(os.path.exists(live))
        self.assertTrue(os.path.exists(foreign))

    def make_hidden(self, name, pid):
        d = os.path.join(self.scratch, name)
        os.makedirs(os.path.join(d, "agents", "t"))
        with open(os.path.join(d, isolation.OWNER_FILE), "w") as f:
            f.write(str(pid))
        os.chmod(os.path.join(d, "agents"), 0o300)
        return d


class Version(unittest.TestCase):
    def test_the_minimum_claude_code_version_for_the_sandbox_settings(self):
        self.assertIn("2.1.268", isolation.version_problem("2.1.200"))
        self.assertIsNone(isolation.version_problem("2.1.268"))
        self.assertIsNone(isolation.version_problem("2.1.331"))
        self.assertIsNone(isolation.version_problem("2.2.0"))
        self.assertIn("cannot tell", isolation.version_problem(None))

    def test_main_refuses_a_real_run_below_the_minimum_in_sandbox_mode_only(self):
        a = mock.Mock(isolation="sandbox")
        d = mock.Mock(version=lambda: "2.1.100")
        self.assertIn("2.1.268", run.isolation_problem(a, d))
        a.isolation = "layout"
        self.assertIsNone(run.isolation_problem(a, d))


class Record(Base):
    def test_the_setup_failed_skeleton_has_the_isolation_field_and_the_schema_checks_it(self):
        with mock.patch("fixture.Fixture.materialise", side_effect=RuntimeError("boom")):
            try:
                self.run_with(EscapeDriver())
            except Exception:
                pass
        rec = self.record()
        self.assertIn("isolation", rec)
        self.assertIsNone(rec["isolation"])
        self.assertEqual(schema.validate_record(rec), [])

    def test_a_bad_isolation_object_is_a_schema_error(self):
        rec = json.load(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "data", "record-ok.json"))) if False else None
        from helpers import make_record
        r = make_record()
        r["isolation"] = {"mode": "sandbox"}
        self.assertTrue([e for e in schema.validate_record(r) if "isolation" in e])
        r["isolation"] = {"mode": "layout", "checked_calls": 3, "violation_count": 0, "violations": []}
        self.assertEqual([e for e in schema.validate_record(r) if "isolation" in e], [])


if __name__ == "__main__":
    unittest.main()
