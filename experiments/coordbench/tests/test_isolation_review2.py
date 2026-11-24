"""Second review of #1538: runs under the home directory, the wildcard root regex, the startup sweep through the entry point,
interrupted runs, the scan's .sh and /bin/ps cases and a pyenv-style python shim."""
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
import time
import unittest
from unittest import mock

from helpers import BENCH  # noqa: F401
import isolation
import run
from test_isolation_run import Base, EscapeDriver
from test_isolation_scan import bash, call


class UnderHome(Base):
    def setUp(self):
        super().setUp()
        self.under = os.path.join(self.home, "cbtmp")
        os.mkdir(self.under, 0o700)

    def test_a_root_under_the_home_is_refused_in_sandbox_mode_with_the_safe_default_named(self):
        msg = isolation.validate_dir(self.under, "--tmp-root", strict=True)
        self.assertIn("lies under", msg)
        self.assertIn(os.path.realpath(tempfile.gettempdir()), msg)
        self.assertIsNone(isolation.validate_dir(self.under, "--tmp-root"))
        self.assertIsNone(isolation.validate_dir(self.tmp, "--tmp-root", strict=True))

    def test_the_other_user_roots_are_refused_too(self):
        for p in ("/Users/someone/cbtmp", "/home/someone/cbtmp", "/root/cbtmp"):
            self.assertIn("lies under", isolation.validate_dir(p, "--logs-dir", strict=True), p)

    def test_main_refuses_a_real_sandboxed_run_with_a_root_under_the_home_and_accepts_layout_mode(self):
        for flag in ("tmp_root", "logs_dir"):
            err = io.StringIO()
            with contextlib.redirect_stderr(err):
                code = run.main(self.argv("B0", **{flag: self.under, "driver": "claude", "model": "m", "isolation": "sandbox"}))
            self.assertEqual(code, 2, flag)
            self.assertIn("lies under", err.getvalue())
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            run.main(self.argv("B0", tmp_root=self.under, driver="claude", model="m", isolation="layout"))
        self.assertNotIn("lies under", err.getvalue())  # refused for the missing budget approval, not for the root

    def test_a_fake_run_keeps_working_under_the_home(self):
        with contextlib.redirect_stdout(io.StringIO()):
            code = run.main(self.argv("B0", tmp_root=self.under))
        self.assertIn(code, (0, 1))
        self.assertTrue(os.path.exists(os.path.join(self.results, "t1")))


class Scan(unittest.TestCase):
    def setUp(self):
        self.policy = isolation.Policy("a", ("/t/s/ta/clone", "/t/s/ta/home", "/t/s/ta/tmp"), guard=["/t/s"])

    def scan(self, *lines):
        return isolation.scan_lines(list(lines), self.policy)[1]

    def test_a_wildcard_root_that_only_the_wildcard_rule_catches(self):
        v = self.scan(bash("ls /Us*"))
        self.assertEqual([x["why"] for x in v], [isolation.ROOT_SEARCH])
        self.assertTrue(self.scan(bash("cat /*/hosanglee/x")))
        self.assertEqual(self.scan(bash("ls /usr/lib/python3*")), [])

    def test_a_file_named_build_sh_is_not_a_shell(self):
        self.assertEqual(self.scan(bash("cat > build.sh <<'EOF'\nimport os\nos.listdir('../c')\nEOF\nchmod +x build.sh")), [])
        self.assertEqual(self.scan(bash("tee run.bash <<'EOF'\ncat ../x\nEOF")), [])
        self.assertTrue(self.scan(bash("bash <<'EOF'\ncat ../c/x\nEOF")))
        self.assertTrue(self.scan(bash("sh -c 'cat ../c/x'")))

    def test_a_path_prefix_on_ps_and_the_disk_wide_finders(self):
        for cmd in ("/bin/ps aux", "/usr/bin/pgrep -a claude", "mdfind SECRET", "locate SPEC-ROUTING", "/usr/bin/mdfind -name x"):
            self.assertTrue(self.scan(bash(cmd)), cmd)
        self.assertEqual(self.scan(bash("git commit -m 'adds locate support'")), [])


class PythonShim(unittest.TestCase):
    def test_the_real_interpreter_behind_a_shim_is_allowed_to_be_read(self):
        tmp = os.path.realpath(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(tmp, ignore_errors=True))
        real = os.path.join(tmp, "versions", "3.13.1")
        os.makedirs(os.path.join(real, "bin"))
        os.makedirs(os.path.join(tmp, "shims"))
        py = os.path.join(real, "bin", "python3")
        with open(py, "w") as f:  # prints what `import sys; print(sys.base_prefix); print(sys.executable)` would
            f.write("#!/bin/sh\necho %s\necho %s\n" % (real, py))
        shim = os.path.join(tmp, "shims", "python3")
        with open(shim, "w") as f:
            f.write("#!/bin/sh\nexec %s \"$@\"\n" % py)
        for p in (py, shim):
            os.chmod(p, 0o755)
        with mock.patch.dict(os.environ, {"PATH": os.path.join(tmp, "shims") + os.pathsep + os.environ["PATH"]}):
            roots = isolation.tool_roots()
        self.assertIn(real, roots)
        self.assertIn(os.path.join(tmp, "shims"), roots)  # the shim's own directory stays allowed

    def test_a_python_that_cannot_be_asked_does_not_break_the_roots(self):
        tmp = os.path.realpath(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(tmp, ignore_errors=True))
        bad = os.path.join(tmp, "python3")
        with open(bad, "w") as f:
            f.write("#!/bin/sh\nexit 3\n")
        os.chmod(bad, 0o755)
        with mock.patch.dict(os.environ, {"PATH": tmp + os.pathsep + os.environ["PATH"]}):
            self.assertIsInstance(isolation.tool_roots(), list)


class Interrupted(Base):
    def hang_script(self):
        script = os.path.join(self.tmp, "hang.json")
        with open(script, "w") as f:
            json.dump({"agents": {"a": [{"op": "hang"}], "b": [{"op": "hang"}]}}, f)
        return script

    def started(self):
        runs = [os.path.join(self.scratch, n) for n in os.listdir(self.scratch)] if os.path.isdir(self.scratch) else []
        return any(os.path.isdir(os.path.join(r, "agents")) and stat.S_IMODE(os.stat(os.path.join(r, "agents")).st_mode) == 0o300 for r in runs)

    def test_sigterm_publishes_the_logs_and_leaves_no_staging_directory_or_marker(self):
        argv = [sys.executable, os.path.join(BENCH, "run.py")] + self.argv("B0", fake_script=self.hang_script())
        p = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=dict(os.environ))
        deadline = time.time() + 30
        while time.time() < deadline and not self.started():
            time.sleep(0.1)
        time.sleep(0.5)
        p.send_signal(signal.SIGTERM)
        p.wait(timeout=30)
        names = os.listdir(self.logs)
        self.assertEqual(names, ["t1-trivial-b0-r01"], names)  # no random staging directory, no .live.* marker
        self.assertEqual(stat.S_IMODE(os.stat(os.path.join(self.logs, names[0])).st_mode), 0o700)

    def test_the_startup_sweep_removes_stale_staging_directories_and_markers_of_dead_runs_but_not_live_ones(self):
        os.mkdir(self.logs)
        dead = subprocess.Popen([sys.executable, "-c", "pass"])
        dead.wait()
        mk = lambda tok, pid, mode: self.stage(tok, pid, mode)
        gone, kept = mk("deadbeefdeadbeef", dead.pid, 0o300), mk("livelivelivelive", os.getpid(), 0o300)
        isolation.sweep_logs(self.logs)
        self.assertFalse(os.path.exists(gone))
        self.assertFalse(os.path.exists(os.path.join(self.logs, ".live.deadbeefdeadbeef")))
        self.assertTrue(os.path.exists(kept))
        self.assertTrue(os.path.exists(os.path.join(self.logs, ".live.livelivelivelive")))
        os.chmod(kept, 0o700)

    def stage(self, tok, pid, mode):
        d = os.path.join(self.logs, tok)
        os.mkdir(d)
        with open(os.path.join(d, "x.log"), "w") as f:
            f.write("x")
        with open(os.path.join(self.logs, ".live." + tok), "w") as f:
            f.write("%d\n" % pid)
        os.chmod(d, mode)
        return d


class SweepAtStart(Base):
    def test_the_suite_entry_point_removes_a_stale_scratch_left_in_the_tmp_root(self):
        dead = subprocess.Popen([sys.executable, "-c", "pass"])
        dead.wait()
        stale = os.path.join(self.scratch, "cb-old-run-abc")
        os.makedirs(os.path.join(stale, "agents", "t"))
        with open(os.path.join(stale, isolation.OWNER_FILE), "w") as f:
            f.write("%d\n" % dead.pid)
        os.chmod(os.path.join(stale, "agents"), 0o300)
        with contextlib.redirect_stdout(io.StringIO()):
            run.main(self.argv("B0", max_run_usd=5, max_suite_usd=50))
        self.assertFalse(os.path.exists(stale))


if __name__ == "__main__":
    unittest.main()
