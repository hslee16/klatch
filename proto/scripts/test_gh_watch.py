"""Tests for proto/scripts/gh-watch-receiver.py (#1329). Run with: python3 -I proto/scripts/test_gh_watch.py

The fixtures are written by hand from GitHub's documented webhook payload shapes (only the fields the digest reads).
Nothing here calls GitHub, creates a webhook or runs `gh`: the receiver runs in a subprocess on an ephemeral port and
is driven with urllib. Every receiver started is killed with SIGKILL in tearDown."""
import ast
import hashlib
import hmac
import importlib.util
import json
import os
import select
import socket
import signal
import stat
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
RECEIVER = os.path.join(HERE, "gh-watch-receiver.py")
LAUNCHER = os.path.join(HERE, "gh-watch")
SECRET = b"test-secret-not-a-credential"

spec = importlib.util.spec_from_file_location("ghw", RECEIVER)
ghw = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ghw)

REPO = {"full_name": "hslee16/klatch", "default_branch": "main"}
SHA = "0123456789abcdef0123456789abcdef01234567"


def pr(number=7, action="opened", **kw):
    p = {"number": number, "title": "Add the watcher", "draft": False, "state": "open", "merged": False,
         "mergeable_state": "clean", "user": {"login": "alice"}, "head": {"ref": "alee/1329-gh-watch", "sha": SHA},
         "base": {"ref": "main"}}
    p.update(kw)
    return {"action": action, "number": number, "pull_request": p, "repository": REPO}


def check_run(conclusion="failure", branch="main", prs=(), name="test"):
    return {"action": "completed", "repository": REPO,
            "check_run": {"name": name, "status": "completed", "conclusion": conclusion, "head_sha": SHA,
                          "check_suite": {"head_branch": branch},
                          "pull_requests": [{"number": n} for n in prs]}}


def workflow_run(conclusion="failure", branch="main", prs=(), name="CI"):
    return {"action": "completed", "repository": REPO,
            "workflow_run": {"name": name, "status": "completed", "conclusion": conclusion, "head_branch": branch,
                             "head_sha": SHA, "pull_requests": [{"number": n} for n in prs]}}


def push(ref="refs/heads/main", after=SHA, deleted=False):
    return {"ref": ref, "after": after, "deleted": deleted, "pusher": {"name": "alice"},
            "commits": [{"message": "a"}, {"message": "b"}], "repository": REPO}


def issue(action="opened", number=9, label=None):
    p = {"action": action, "issue": {"number": number, "title": "A bug", "user": {"login": "bob"}},
         "repository": REPO}
    if label:
        p["label"] = {"name": label}
    return p


class DigestTests(unittest.TestCase):
    def d(self, event, payload, state=None):
        return ghw.digest(event, payload, state if state is not None else ghw.State())

    def test_pr_opened(self):
        (sev, text), = self.d("pull_request", pr())
        self.assertEqual(sev, "INFO")
        self.assertIn("PR #7 opened", text)
        self.assertIn("alice", text)

    def test_pr_synchronize_ready_closed_merged(self):
        (sev, text), = self.d("pull_request", pr(action="synchronize"))
        self.assertEqual((sev, "synchronize" in text or "updated" in text), ("INFO", True))
        (sev, text), = self.d("pull_request", pr(action="ready_for_review"))
        self.assertEqual(sev, "INFO")
        self.assertIn("ready for review", text)
        (sev, text), = self.d("pull_request", pr(action="closed", state="closed", merged=True))
        self.assertEqual(sev, "INFO")
        self.assertIn("merged", text)
        (sev, text), = self.d("pull_request", pr(action="closed", state="closed", merged=False))
        self.assertIn("closed", text)
        self.assertNotIn("merged", text)

    def test_pr_dirty_is_alert(self):
        out = self.d("pull_request", pr(action="synchronize", mergeable_state="dirty"))
        self.assertIn("ALERT", [s for s, _ in out])
        alert = [t for s, t in out if s == "ALERT"][0]
        self.assertIn("#7", alert)
        self.assertIn("conflict", alert)

    def test_pr_mergeable_state_null_is_not_alert(self):
        out = self.d("pull_request", pr(mergeable_state=None))
        self.assertEqual([s for s, _ in out], ["INFO"])

    def test_check_run_failure_on_main_is_alert(self):
        (sev, text), = self.d("check_run", check_run("failure", "main"))
        self.assertEqual(sev, "ALERT")
        self.assertIn("main", text)
        (sev, _), = self.d("check_run", check_run("timed_out", "main"))
        self.assertEqual(sev, "ALERT")

    def test_check_run_success_is_silent(self):
        self.assertEqual(self.d("check_run", check_run("success", "main")), [])

    def test_check_run_failure_on_open_pr_is_alert_but_not_on_a_draft(self):
        st = ghw.State()
        (sev, text), = self.d("check_run", check_run("failure", "alee/x", prs=[7]), st)
        self.assertEqual(sev, "ALERT")
        self.assertIn("#7", text)
        self.d("pull_request", pr(7, "converted_to_draft", draft=True), st)
        self.assertEqual(self.d("check_run", check_run("failure", "alee/x", prs=[7]), st), [])
        self.d("pull_request", pr(7, "ready_for_review", draft=False), st)
        self.assertEqual(len(self.d("check_run", check_run("failure", "alee/x", prs=[7]), st)), 1)

    def test_check_run_failure_on_a_branch_without_pr_is_silent(self):
        self.assertEqual(self.d("check_run", check_run("failure", "scratch")), [])

    def test_workflow_run_failure_on_main_and_on_a_branch(self):
        (sev, text), = self.d("workflow_run", workflow_run("failure", "main"))
        self.assertEqual(sev, "ALERT")
        self.assertIn("main", text)
        (sev, text), = self.d("workflow_run", workflow_run("failure", "alee/x", prs=[7]))
        self.assertEqual(sev, "ALERT")
        self.assertIn("#7", text)
        self.assertEqual(self.d("workflow_run", workflow_run("failure", "scratch")), [])
        self.assertEqual(self.d("workflow_run", workflow_run("success", "main")), [])

    def test_push_to_main(self):
        (sev, text), = self.d("push", push())
        self.assertEqual(sev, "INFO")
        self.assertIn("main moved to " + SHA[:7], text)
        self.assertIn("open PRs may now conflict", text)

    def test_push_elsewhere_or_deleted_is_silent(self):
        self.assertEqual(self.d("push", push("refs/heads/alee/x")), [])
        self.assertEqual(self.d("push", push(deleted=True)), [])

    def test_issues(self):
        (sev, text), = self.d("issues", issue())
        self.assertEqual(sev, "INFO")
        self.assertIn("issue #9 opened", text)
        (sev, text), = self.d("issues", issue("labeled", label="bug"))
        self.assertIn("labeled", text)
        self.assertIn("bug", text)
        self.assertEqual(self.d("issues", issue("closed")), [])

    def test_unknown_event_and_issue_comment_emit_nothing(self):
        self.assertEqual(self.d("star", {"action": "created"}), [])
        self.assertEqual(self.d("issue_comment", {"action": "created", "comment": {"body": "hi"}}), [])

    def test_control_characters_stripped_and_truncated(self):
        evil = "fix\x1b[31m red\r\nALERT forged\x00‮" + "x" * 500
        out = self.d("pull_request", pr(title=evil))
        for _, text in out:
            self.assertNotRegex(text, r"[\x00-\x1f\x7f‮ ]")
            self.assertLessEqual(len(text), 400)

    def test_garbage_payload_does_not_raise(self):
        self.assertEqual(self.d("pull_request", {"action": "opened"}), [])
        self.assertEqual(self.d("push", "not a dict"), [])


class SourceTests(unittest.TestCase):
    """The watcher is read-only: no gh call, no GitHub API, no HTTP client (#1329)."""

    def test_no_gh_invocation_or_http_client(self):
        with open(RECEIVER, encoding="utf-8") as f:
            src = f.read()
        tree = ast.parse(src)
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                names = [a.name for a in node.names]
            elif isinstance(node, ast.ImportFrom):
                names = [node.module or ""]
            else:
                names = []
            for n in names:
                for bad in ("urllib.request", "http.client", "requests", "httpx", "urllib3", "aiohttp", "ftplib",
                            "smtplib", "telnetlib"):
                    self.assertFalse(n == bad or n.startswith(bad + "."), "imports " + n)
            if isinstance(node, ast.Constant) and isinstance(node.value, str):
                v = node.value.strip()
                self.assertFalse(v == "gh" or v.startswith("gh "), "string looks like a gh call: " + v)
                self.assertNotIn("api.github.com", v)
        calls = [n for n in ast.walk(tree) if isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute)
                 and isinstance(n.func.value, ast.Name) and n.func.value.id == "subprocess"]
        self.assertEqual([c.func.attr for c in calls], ["run"], "the only subprocess use is the one klatch post")
        for word in ("os.system", "os.popen", "eval(", "exec(", "shell=True"):
            self.assertNotIn(word, src.replace("never " + word, ""))

    def test_launcher_forward_command_prints_and_runs_nothing(self):
        with tempfile.TemporaryDirectory() as t:
            rec = os.path.join(t, "ran")
            fake = os.path.join(t, "bin")
            os.mkdir(fake)
            with open(os.path.join(fake, "gh"), "w") as f:
                f.write('#!/bin/sh\necho "$@" >> "%s"\nexit 0\n' % rec)
            os.chmod(os.path.join(fake, "gh"), 0o755)
            env = {"PATH": fake + ":/usr/bin:/bin", "HOME": t, "TMPDIR": t}
            r = subprocess.run([LAUNCHER, "forward-command", "--out-dir", t], env=env, capture_output=True,
                               text=True, timeout=30)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertIn("gh auth refresh -s admin:repo_hook", r.stdout)
            self.assertIn("gh webhook forward", r.stdout)
            self.assertIn("--repo=hslee16/klatch", r.stdout)
            self.assertIn("--events=pull_request,check_run,workflow_run,push,issues,issue_comment", r.stdout)
            self.assertIn("http://127.0.0.1:", r.stdout)
            self.assertFalse(os.path.exists(rec), "forward-command ran gh")

    def test_launcher_refuses_out_dir_in_klatch_home(self):
        with tempfile.TemporaryDirectory() as t:
            env = {"PATH": "/usr/bin:/bin", "HOME": t, "TMPDIR": t}
            r = subprocess.run([LAUNCHER, "serve", "--out-dir", os.path.join(t, ".klatch", "x")], env=env,
                               capture_output=True, text=True, timeout=30)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn(".klatch", r.stderr)


class LauncherStateTests(unittest.TestCase):
    """`gh-watch status` and `gh-watch ensure` (#1335): fake ps, a temporary state dir, an ephemeral port, no gh."""

    FORWARDER = ("gh webhook forward --repo=hslee16/klatch --events=pull_request --url=http://127.0.0.1:%d/hook "
                 "--secret TOPSECRETVALUE")

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.t = self.tmp.name
        self.state = os.path.join(self.t, "state")
        self.bin = os.path.join(self.t, "bin")
        os.mkdir(self.bin)
        self.ps_out = os.path.join(self.t, "ps.out")
        self.gh_rec = os.path.join(self.t, "gh.ran")
        self.set_ps([])
        for name, body in (("ps", '#!/bin/sh\ncat "%s"\n' % self.ps_out),
                           ("gh", '#!/bin/sh\necho "$@" >> "%s"\nexit 0\n' % self.gh_rec)):
            with open(os.path.join(self.bin, name), "w") as f:
                f.write(body)
            os.chmod(os.path.join(self.bin, name), 0o755)
        s = socket.socket()
        s.bind(("127.0.0.1", 0))
        self.port = s.getsockname()[1]
        s.close()

    def tearDown(self):
        self.kill_receiver()
        p = getattr(self, "serve_proc", None)
        if p:
            p.wait(timeout=10)
        self.tmp.cleanup()

    def set_ps(self, lines):
        with open(self.ps_out, "w") as f:
            f.write("".join(l + "\n" for l in ["/sbin/launchd", "-zsh", "python3 -I x.py"] + list(lines)))

    def pid(self):
        try:
            with open(os.path.join(self.state, "gh-watch.pid")) as f:
                return int(f.read().strip())
        except (OSError, ValueError):
            return None

    def kill_receiver(self):
        pid = self.pid()
        if pid:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass

    def run_cmd(self, *args):
        env = {"PATH": self.bin + ":/usr/bin:/bin", "HOME": self.t, "TMPDIR": self.t}
        return subprocess.run([LAUNCHER, *args, "--out-dir", self.state, "--port", str(self.port)], env=env,
                              capture_output=True, text=True, timeout=60)

    def make_secret(self):
        os.makedirs(self.state, mode=0o700, exist_ok=True)
        with open(os.path.join(self.state, "secret"), "w") as f:
            f.write("TOPSECRETVALUE\n")
        os.chmod(os.path.join(self.state, "secret"), 0o600)

    def wait_for(self, fn, what):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if fn():
                return
            time.sleep(0.05)
        self.fail("timed out waiting for " + what)

    def start_serve(self):
        self.make_secret()
        env = {"PATH": self.bin + ":/usr/bin:/bin", "HOME": self.t, "TMPDIR": self.t}
        self.serve_proc = subprocess.Popen([LAUNCHER, "serve", "--out-dir", self.state, "--port", str(self.port)], env=env,
                         stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.wait_for(lambda: self.run_cmd("status").returncode != 3, "the receiver to listen")

    def test_status_receiver_down(self):
        self.set_ps([self.FORWARDER % self.port])
        r = self.run_cmd("status")
        self.assertEqual(r.returncode, 3, r.stdout + r.stderr)
        self.assertEqual(r.stdout.strip().splitlines()[-1], "gh-watch: receiver down")

    def test_status_stale_pid_file_is_down(self):
        os.makedirs(self.state, mode=0o700)
        dead = subprocess.Popen(["/usr/bin/true"])
        dead.wait()
        with open(os.path.join(self.state, "gh-watch.pid"), "w") as f:
            f.write("%d\n" % dead.pid)
        r = self.run_cmd("status")
        self.assertEqual(r.returncode, 3)

    def test_status_ok_hides_forwarder_arguments(self):
        self.start_serve()
        self.assertIsNotNone(self.pid(), "serve wrote no pid file")
        self.set_ps([self.FORWARDER % self.port])
        r = self.run_cmd("status")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(r.stdout.strip().splitlines()[-1], "gh-watch: ok")
        for word in ("TOPSECRETVALUE", "--secret", "--events"):
            self.assertNotIn(word, r.stdout + r.stderr)
        self.assertFalse(os.path.exists(self.gh_rec), "status ran gh")

    def test_status_forwarder_down(self):
        self.start_serve()
        for lines in ([], ["gh webhook forward --repo=someone/else --secret X"], ["vim gh-webhook-forward-notes"]):
            self.set_ps(lines)
            r = self.run_cmd("status")
            self.assertEqual(r.returncode, 4, r.stdout + r.stderr)
            self.assertEqual(r.stdout.strip().splitlines()[-1], "gh-watch: forwarder down")
        self.assertFalse(os.path.exists(self.gh_rec))

    def test_status_forwarder_down_stale_webhook_is_exit_5(self):
        self.start_serve()
        with open(os.path.join(self.state, "forwarder.stale"), "w") as f:
            f.write("stale webhook on the repository: run gh-watch clean-hooks\n")
        self.set_ps([])
        r = self.run_cmd("status")
        self.assertEqual(r.returncode, 5, r.stdout + r.stderr)
        self.assertEqual(r.stdout.strip().splitlines()[-1], "gh-watch: forwarder down: stale webhook")
        self.assertIn("gh-watch clean-hooks", r.stdout)
        # a running forwarder wins over a leftover marker
        self.set_ps([self.FORWARDER % self.port])
        r = self.run_cmd("status")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertFalse(os.path.exists(self.gh_rec), "status ran gh")

    def test_status_receiver_down_beats_stale_marker(self):
        os.makedirs(self.state, mode=0o700)
        with open(os.path.join(self.state, "forwarder.stale"), "w") as f:
            f.write("x\n")
        r = self.run_cmd("status")
        self.assertEqual(r.returncode, 3, r.stdout + r.stderr)

    def test_status_reports_log_age(self):
        self.start_serve()
        r = self.run_cmd("status")
        self.assertIn("no log lines", r.stdout)
        with open(os.path.join(self.state, "gh-watch.log"), "a") as f:
            f.write("2026-11-22T10:00:00Z INFO hello\n")
        os.utime(os.path.join(self.state, "gh-watch.log"), (time.time() - 120, time.time() - 120))
        r = self.run_cmd("status")
        self.assertRegex(r.stdout, r"last log line: 1[12]\d s ago")

    def test_serve_removes_pid_file_when_receiver_dies(self):
        self.start_serve()
        pid = self.pid()
        os.kill(pid, signal.SIGKILL)
        self.wait_for(lambda: not os.path.exists(os.path.join(self.state, "gh-watch.pid")), "the pid file to go")
        self.assertEqual(self.run_cmd("status").returncode, 3)

    def test_ensure_starts_receiver_only_and_hints_forwarder(self):
        r = self.run_cmd("ensure")
        self.assertEqual(r.returncode, 4, r.stdout + r.stderr)
        self.assertEqual(os.path.isdir(self.state), True)
        self.assertEqual(stat.S_IMODE(os.stat(os.path.join(self.state, "secret")).st_mode), 0o600)
        with open(os.path.join(self.state, "secret")) as f:
            secret = f.read().strip()
        self.assertGreaterEqual(len(secret), 32)
        self.assertNotIn(secret, r.stdout + r.stderr)
        self.assertIn("forward-command", r.stdout)
        self.assertTrue(os.path.exists(os.path.join(self.state, "serve.out")))
        pid = self.pid()
        self.assertIsNotNone(pid)
        self.assertEqual(self.run_cmd("status").returncode, 4, "receiver should be up")
        self.assertFalse(os.path.exists(self.gh_rec), "ensure ran gh")
        r2 = self.run_cmd("ensure")
        self.assertEqual(r2.returncode, 4)
        self.assertEqual(self.pid(), pid, "a second receiver was started")
        with open(os.path.join(self.state, "secret")) as f:
            self.assertEqual(f.read().strip(), secret, "ensure replaced the secret")

    def test_ensure_ok_when_forwarder_present(self):
        self.set_ps([self.FORWARDER % self.port])
        r = self.run_cmd("ensure")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(r.stdout.strip().splitlines()[-1], "gh-watch: ok")
        self.assertNotIn("TOPSECRETVALUE", r.stdout)

    def test_ensure_source_never_names_gh(self):
        with open(LAUNCHER) as f:
            src = f.read()
        self.assertIn("cmd_ensure()", src)
        body = src.split("cmd_ensure()")[1].split("\n}\n")[0]
        for line in body.splitlines():
            code = line.split("#")[0]
            self.assertNotRegex(code, r"(^|[\s;|&(])gh\s", "ensure must not run gh: " + line)
            self.assertNotIn("webhook forward", code.replace("forward-command", ""))


class ReceiverTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.t = self.tmp.name
        self.secret_file = os.path.join(self.t, "secret")
        self.write_secret(0o600)
        self.out = os.path.join(self.t, "out")
        self.procs = []

    def tearDown(self):
        for p in self.procs:
            try:
                os.kill(p.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            p.wait(timeout=10)
            p.stdout.close()
            p.stderr.close()
        self.tmp.cleanup()

    def write_secret(self, mode):
        with open(self.secret_file, "wb") as f:
            f.write(SECRET + b"\n")
        os.chmod(self.secret_file, mode)

    def start(self, *extra, bind=None, expect_ready=True):
        cmd = [sys.executable, "-I", RECEIVER, "--secret-file", self.secret_file, "--out-dir", self.out,
               "--port", "0"] + (["--bind", bind] if bind else []) + list(extra)
        p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                             env={"PATH": "/usr/bin:/bin", "HOME": self.t})
        self.procs.append(p)
        if not expect_ready:
            return p, None
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            r, _, _ = select.select([p.stdout], [], [], 0.2)
            if r:
                line = p.stdout.readline()
                if line.startswith("listening 127.0.0.1:"):
                    return p, int(line.rsplit(":", 1)[1])
                break
            if p.poll() is not None:
                break
        self.fail("receiver not ready: " + (p.stderr.read() if p.poll() is not None else "timeout"))

    def post(self, port, event, payload, delivery="d1", sig="auto", raw=None, secret=SECRET):
        body = raw if raw is not None else json.dumps(payload).encode()
        h = {"X-GitHub-Event": event, "X-GitHub-Delivery": delivery, "Content-Type": "application/json"}
        if sig == "auto":
            h["X-Hub-Signature-256"] = "sha256=" + hmac.new(secret, body, hashlib.sha256).hexdigest()
        elif sig is not None:
            h["X-Hub-Signature-256"] = sig
        req = urllib.request.Request("http://127.0.0.1:%d/hook" % port, data=body, headers=h, method="POST")
        try:
            with urllib.request.urlopen(req, timeout=10) as r:
                return r.status, r.read().decode()
        except urllib.error.HTTPError as e:
            with e:
                return e.code, e.read().decode()

    def log(self):
        path = os.path.join(self.out, "gh-watch.log")
        if not os.path.exists(path):
            return []
        with open(path, encoding="utf-8") as f:
            return f.read().splitlines()

    def test_signed_pull_request_is_logged_one_line(self):
        _, port = self.start()
        code, _ = self.post(port, "pull_request", pr())
        self.assertEqual(code, 200)
        lines = self.log()
        self.assertEqual(len(lines), 1)
        self.assertIn("INFO", lines[0])
        self.assertIn("PR #7 opened", lines[0])

    def test_each_fixture_event_over_http(self):
        _, port = self.start()
        cases = [("pull_request", pr(action="synchronize"), "INFO"),
                 ("pull_request", pr(7, "closed", state="closed", merged=True), "INFO"),
                 ("pull_request", pr(7, "ready_for_review"), "INFO"),
                 ("check_run", check_run(), "ALERT"),
                 ("workflow_run", workflow_run(), "ALERT"),
                 ("workflow_run", workflow_run(branch="alee/x", prs=[7]), "ALERT"),
                 ("push", push(), "INFO"),
                 ("issues", issue(), "INFO")]
        for i, (ev, pl, sev) in enumerate(cases):
            self.assertEqual(self.post(port, ev, pl, delivery="e%d" % i)[0], 200)
        got = [l.split()[1] for l in self.log()]
        self.assertEqual(got, [c[2] for c in cases])

    def test_bad_signature_refused(self):
        _, port = self.start()
        code, _ = self.post(port, "pull_request", pr(), secret=b"wrong")
        self.assertEqual(code, 401)
        self.assertEqual(self.log(), [])

    def test_missing_signature_refused(self):
        _, port = self.start()
        code, _ = self.post(port, "pull_request", pr(), sig=None)
        self.assertEqual(code, 401)
        code, _ = self.post(port, "pull_request", pr(), sig="sha1=abcd")
        self.assertEqual(code, 401)
        self.assertEqual(self.log(), [])

    def test_oversized_body_refused(self):
        _, port = self.start("--max-body", "2048")
        big = json.dumps({"pad": "x" * 5000}).encode()
        code, _ = self.post(port, "push", None, raw=big)
        self.assertEqual(code, 413)
        self.assertEqual(self.log(), [])

    def test_duplicate_delivery_ignored(self):
        _, port = self.start()
        self.assertEqual(self.post(port, "pull_request", pr(), delivery="same")[0], 200)
        code, text = self.post(port, "pull_request", pr(), delivery="same")
        self.assertEqual(code, 200)
        self.assertIn("duplicate", text)
        self.assertEqual(len(self.log()), 1)

    def test_bad_signature_does_not_burn_the_delivery_id(self):
        _, port = self.start()
        self.assertEqual(self.post(port, "pull_request", pr(), delivery="x", secret=b"wrong")[0], 401)
        self.assertEqual(self.post(port, "pull_request", pr(), delivery="x")[0], 200)
        self.assertEqual(len(self.log()), 1)

    def test_unknown_event_ignored(self):
        _, port = self.start()
        code, text = self.post(port, "star", {"action": "created"})
        self.assertEqual(code, 200)
        self.assertIn("ignored", text)
        self.assertEqual(self.log(), [])

    def test_unsigned_unknown_event_still_refused(self):
        _, port = self.start()
        self.assertEqual(self.post(port, "star", {}, sig=None)[0], 401)

    def test_non_loopback_bind_refused(self):
        for addr in ("0.0.0.0", "192.168.1.5", "::", "localhost.example.com"):
            p, _ = self.start(bind=addr, expect_ready=False)
            try:
                p.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.fail("receiver stayed up on " + addr)
            self.assertNotEqual(p.returncode, 0)
            self.assertIn("loopback", p.stderr.read())

    def test_world_or_group_readable_secret_refused(self):
        for mode in (0o644, 0o640, 0o604, 0o660):
            self.write_secret(mode)
            p, _ = self.start(expect_ready=False)
            p.wait(timeout=15)
            self.assertNotEqual(p.returncode, 0, oct(mode))
            self.assertIn("0600", p.stderr.read())

    def test_missing_or_empty_secret_refused(self):
        os.remove(self.secret_file)
        p, _ = self.start(expect_ready=False)
        p.wait(timeout=15)
        self.assertNotEqual(p.returncode, 0)
        with open(self.secret_file, "w"):
            pass
        os.chmod(self.secret_file, 0o600)
        p, _ = self.start(expect_ready=False)
        p.wait(timeout=15)
        self.assertNotEqual(p.returncode, 0)

    def test_control_characters_never_reach_the_log(self):
        _, port = self.start()
        self.post(port, "pull_request", pr(title="a\x1b[2J\r\nALERT forged line\x07"))
        lines = self.log()
        self.assertEqual(len(lines), 1)
        with open(os.path.join(self.out, "gh-watch.log"), "rb") as f:
            raw = f.read()
        self.assertNotIn(b"\x1b", raw)
        self.assertNotIn(b"\x07", raw)
        self.assertNotIn(b"\r", raw)

    def test_other_path_and_method_refused(self):
        _, port = self.start()
        req = urllib.request.Request("http://127.0.0.1:%d/other" % port, data=b"{}", method="POST")
        for r, want in ((req, 404), ("http://127.0.0.1:%d/hook" % port, 405)):
            with self.assertRaises(urllib.error.HTTPError) as cm:
                urllib.request.urlopen(r, timeout=10)
            self.assertEqual(cm.exception.code, want)
            cm.exception.close()

    def test_post_to_klatch_room_with_a_fake_binary(self):
        rec = os.path.join(self.t, "calls")
        fake = os.path.join(self.t, "klatch")
        with open(fake, "w") as f:
            f.write('#!/bin/sh\nprintf "%%s|%%s|%%s|%%s\\n" "$KLATCH_NAME" "$1" "$2" "$3" >> "%s"\n' % rec)
        os.chmod(fake, 0o755)
        _, port = self.start("--klatch-bin", fake, "--as-member", "ghwatch", "--room", "factory-status")
        self.post(port, "check_run", check_run(), delivery="k1")
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and not os.path.exists(rec):
            time.sleep(0.05)  # bounded poll, in the test process only
        with open(rec) as f:
            call = [c for c in (l.split("|") for l in f.read().splitlines()) if c[1] == "post"][0]
        self.assertEqual(call[:3], ["ghwatch", "post", "factory-status"])
        self.assertIn("ALERT", call[3])

    def test_klatch_post_failure_does_not_stop_the_log(self):
        _, port = self.start("--klatch-bin", "/nonexistent/klatch", "--as-member", "m", "--room", "r")
        self.assertEqual(self.post(port, "check_run", check_run())[0], 200)
        self.assertEqual(len(self.log()), 1)

    def test_klatch_bin_needs_member_and_room(self):
        p, _ = self.start("--klatch-bin", "/bin/true", expect_ready=False)
        p.wait(timeout=15)
        self.assertNotEqual(p.returncode, 0)

    # --- the watcher claim (#1338), through a fake klatch binary ---------------------------------------------------
    def fake_klatch(self, refuse=False):
        """A klatch that records `NAME|args` per call; `claim` answers `refused:` while the file `refuse` exists."""
        self.calls_file = os.path.join(self.t, "calls")
        self.refuse_file = os.path.join(self.t, "refuse")
        if refuse:
            open(self.refuse_file, "w").close()
        fake = os.path.join(self.t, "klatch")
        with open(fake, "w") as f:
            f.write('#!/bin/sh\nprintf "%%s|%%s\\n" "$KLATCH_NAME" "$*" >> "%s"\n'
                    'if [ "$1" = claim ] && [ -e "%s" ]; then echo "refused: watch:gh is claimed by o for another 5s" >&2;'
                    ' exit 1; fi\n' % (self.calls_file, self.refuse_file))
        os.chmod(fake, 0o755)
        return fake

    def klatch_calls(self):
        if not os.path.exists(self.calls_file):
            return []
        with open(self.calls_file) as f:
            return [l.split("|", 1)[1] for l in f.read().splitlines()]

    def wait_for(self, pred, what):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if pred():
                return
            time.sleep(0.05)  # bounded poll, in the test process only
        self.fail("timed out waiting for " + what)

    def test_claim_taken_renewed_and_released_on_clean_stop(self):
        fake = self.fake_klatch()
        p, _ = self.start("--klatch-bin", fake, "--as-member", "ghwatch", "--room", "factory-status",
                          "--claim-ttl", "6", "--claim-renew", "1")
        self.wait_for(lambda: len([c for c in self.klatch_calls() if c.startswith("claim")]) >= 3, "renewals")
        first = self.klatch_calls()[0]
        self.assertEqual(first, "claim factory-status watch:gh --ttl 6 --shared --reason gh-watch receiver")
        p.send_signal(signal.SIGTERM)
        p.wait(timeout=15)
        self.assertEqual(p.returncode, 0)
        self.assertEqual(self.klatch_calls()[-1], "release factory-status watch:gh")

    def test_second_receiver_refused_by_the_claim(self):
        fake = self.fake_klatch(refuse=True)
        p, _ = self.start("--klatch-bin", fake, "--as-member", "ghwatch", "--room", "r", expect_ready=False)
        p.wait(timeout=15)
        self.assertEqual(p.returncode, 2)
        self.assertIn("watch:gh", p.stderr.read())
        self.assertNotIn("listening", p.stdout.read())
        self.assertEqual([c.split()[0] for c in self.klatch_calls()], ["claim"], "no release of the holder's claim")

    def test_kill_9_leaves_the_claim_to_lapse(self):
        fake = self.fake_klatch()
        p, _ = self.start("--klatch-bin", fake, "--as-member", "m", "--room", "r")
        self.wait_for(lambda: self.klatch_calls(), "the first claim")
        p.kill()
        p.wait(timeout=15)
        self.assertNotIn("release", [c.split()[0] for c in self.klatch_calls()])

    def test_no_claim_flag_and_no_klatch_means_no_claim(self):
        fake = self.fake_klatch()
        p, _ = self.start("--klatch-bin", fake, "--as-member", "m", "--room", "r", "--no-claim")
        p.send_signal(signal.SIGTERM)
        p.wait(timeout=15)
        self.assertEqual(self.klatch_calls(), [])
        p2, _ = self.start()
        p2.send_signal(signal.SIGTERM)
        p2.wait(timeout=15)

    def test_renew_must_be_shorter_than_ttl(self):
        p, _ = self.start("--klatch-bin", "/bin/true", "--as-member", "m", "--room", "r", "--claim-ttl", "10",
                          "--claim-renew", "10", expect_ready=False)
        p.wait(timeout=15)
        self.assertEqual(p.returncode, 2)

    def test_claim_failure_other_than_refusal_keeps_the_log_working(self):
        _, port = self.start("--klatch-bin", "/nonexistent/klatch", "--as-member", "m", "--room", "r")
        self.assertEqual(self.post(port, "push", push())[0], 200)
        self.assertEqual(len(self.log()), 1)

    def test_launcher_sigterm_reaches_the_receiver_which_releases(self):
        fake = self.fake_klatch()
        env = {"PATH": "/usr/bin:/bin", "HOME": self.t}
        p = subprocess.Popen([LAUNCHER, "serve", "--out-dir", self.out, "--secret-file", self.secret_file,
                              "--port", "0", "--klatch-bin", fake, "--as-member", "m", "--room", "r"],
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
        self.procs.append(p)
        self.wait_for(lambda: self.klatch_calls(), "the first claim")
        p.send_signal(signal.SIGTERM)
        p.wait(timeout=15)
        self.assertEqual(self.klatch_calls()[-1], "release r watch:gh")

    def test_log_is_flushed_per_line(self):
        _, port = self.start()
        self.post(port, "push", push())
        # read while the receiver is still running: the line must already be on disk
        self.assertEqual(len(self.log()), 1)
        self.assertTrue(stat.S_IMODE(os.stat(self.out).st_mode) & 0o077 == 0, "out dir should be private")


class FakeKlatch:
    """Stands in for the one klatch call: records the argv and answers from a script of (returncode, stderr)."""

    def __init__(self):
        self.calls, self.answers = [], []

    def __call__(self, args):
        self.calls.append(list(args))
        return self.answers.pop(0) if self.answers else (0, "")


class ClaimKeeperTests(unittest.TestCase):
    """The watcher claim (#1338) against a fake clock and a fake klatch: no process, no sleep."""

    REFUSED = (1, "refused: watch:gh is claimed by other for another 50s")

    def setUp(self):
        self.now = 1000.0
        self.k = FakeKlatch()
        self.keeper = ghw.ClaimKeeper(self.k, lambda: self.now, "factory-status", "watch:gh", 90, 30)

    def test_documented_defaults(self):
        self.assertEqual((ghw.CLAIM_NAME, ghw.CLAIM_TTL, ghw.CLAIM_RENEW), ("watch:gh", 90, 30))
        self.assertGreaterEqual(ghw.CLAIM_TTL, 3 * ghw.CLAIM_RENEW, "two renewals may be missed before a lapse")

    def test_acquire_claims_shared_with_the_ttl(self):
        self.assertEqual(self.keeper.acquire(), "held")
        self.assertEqual(self.k.calls, [["claim", "factory-status", "watch:gh", "--ttl", "90", "--shared",
                                         "--reason", "gh-watch receiver"]])

    def test_renews_on_the_interval_and_not_before(self):
        self.keeper.acquire()
        self.now += 29
        self.assertFalse(self.keeper.tick())
        self.assertEqual(len(self.k.calls), 1)
        self.now += 1
        self.assertTrue(self.keeper.tick())
        self.assertEqual(len(self.k.calls), 2)
        self.assertEqual(self.k.calls[1], self.k.calls[0], "a renewal is the same claim again")
        self.now += 15
        self.assertFalse(self.keeper.tick())
        self.now += 15
        self.assertTrue(self.keeper.tick())
        self.assertEqual(len(self.k.calls), 3)

    def test_a_long_stall_renews_once_not_in_a_burst(self):
        self.keeper.acquire()
        self.now += 30 * 10
        self.assertTrue(self.keeper.tick())
        self.assertFalse(self.keeper.tick())
        self.assertEqual(len(self.k.calls), 2)

    def test_second_receiver_is_refused_by_the_claim_conflict(self):
        self.k.answers.append(self.REFUSED)
        self.assertEqual(self.keeper.acquire(), "refused")
        self.keeper.release()
        self.assertEqual([c[0] for c in self.k.calls], ["claim"], "a refused receiver never releases the holder's claim")

    def test_other_failure_is_not_a_refusal_and_is_retried(self):
        self.k.answers.append((1, "the store is busy"))
        self.assertEqual(self.keeper.acquire(), "failed")
        self.now += 30
        self.assertTrue(self.keeper.tick())
        self.assertTrue(self.keeper.held)
        self.keeper.release()
        self.assertEqual(self.k.calls[-1][:3], ["release", "factory-status", "watch:gh"])

    def test_losing_the_claim_stops_the_release(self):
        self.keeper.acquire()
        self.k.answers.append(self.REFUSED)
        self.now += 30
        self.keeper.tick()
        self.assertFalse(self.keeper.held)
        self.keeper.release()
        self.assertEqual([c[0] for c in self.k.calls], ["claim", "claim"])

    def test_release_runs_once(self):
        self.keeper.acquire()
        self.keeper.release()
        self.keeper.release()
        self.assertEqual([c[0] for c in self.k.calls], ["claim", "release"])
        self.assertEqual(self.k.calls[1], ["release", "factory-status", "watch:gh"])

    def test_message_text_is_not_trusted_to_decide(self):
        # only the fixed prefix means a conflict; a refusal word later in the text does not
        self.k.answers.append((1, "boom refused: watch:gh is claimed by x"))
        self.assertEqual(self.keeper.acquire(), "failed")


class LaunchdTests(unittest.TestCase):
    """`gh-watch install-launchd` only PRINTS (#1343); `gh-watch-forward` is the reviewed wrapper the plist points to."""

    RECEIVER_LABEL = "dev.klatch.gh-watch-receiver"
    FORWARDER_LABEL = "dev.klatch.gh-watch-forwarder"
    SECRET_TEXT = "WRAPPERSECRETVALUE"

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.t = self.tmp.name
        self.bin = os.path.join(self.t, "bin")
        os.mkdir(self.bin)
        self.rec = os.path.join(self.t, "ran")
        for name in ("gh", "launchctl"):
            with open(os.path.join(self.bin, name), "w") as f:
                f.write('#!/bin/sh\necho "%s $@" >> "%s"\nexit 0\n' % (name, self.rec))
            os.chmod(os.path.join(self.bin, name), 0o755)
        self.state = os.path.join(self.t, "state")
        self.env = {"PATH": self.bin + ":/usr/bin:/bin", "HOME": self.t, "TMPDIR": self.t}

    def tearDown(self):
        self.tmp.cleanup()

    def install(self, *args):
        r = subprocess.run([LAUNCHER, "install-launchd", "--out-dir", self.state, *args], env=self.env,
                           capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 0, r.stderr)
        return r.stdout

    def plists(self, out):
        blocks = []
        rest = out
        while "<?xml" in rest:
            a = rest.index("<?xml")
            b = rest.index("</plist>", a) + len("</plist>")
            blocks.append(rest[a:b])
            rest = rest[b:]
        return blocks

    def test_prints_two_wellformed_plists(self):
        import plistlib
        out = self.install()
        blocks = self.plists(out)
        self.assertEqual(len(blocks), 2)
        parsed = [plistlib.loads(b.encode()) for b in blocks]
        self.assertEqual(sorted(p["Label"] for p in parsed), [self.FORWARDER_LABEL, self.RECEIVER_LABEL])
        for p in parsed:
            self.assertIs(p["KeepAlive"], True)
            self.assertIs(p["RunAtLoad"], True)
            self.assertGreaterEqual(p["ThrottleInterval"], 10)
            self.assertTrue(p["StandardOutPath"].startswith(self.state + "/"), p["StandardOutPath"])
            self.assertTrue(p["StandardErrorPath"].startswith(self.state + "/"), p["StandardErrorPath"])
            self.assertTrue(os.path.isabs(p["ProgramArguments"][0]))
            self.assertNotIn("EnvironmentVariables.SECRET", str(p))
        by = {p["Label"]: p for p in parsed}
        self.assertEqual(by[self.RECEIVER_LABEL]["ProgramArguments"][:2], [os.path.join(HERE, "gh-watch"), "serve"])
        self.assertEqual(by[self.FORWARDER_LABEL]["ProgramArguments"][0], os.path.join(HERE, "gh-watch-forward"))
        self.assertIn(os.path.join(self.state, "secret"), by[self.RECEIVER_LABEL]["ProgramArguments"])

    def test_plutil_lint_when_available(self):
        import shutil
        if not shutil.which("plutil"):
            self.skipTest("no plutil")
        for i, b in enumerate(self.plists(self.install())):
            path = os.path.join(self.t, "x%d.plist" % i)
            with open(path, "w") as f:
                f.write(b)
            r = subprocess.run(["plutil", "-lint", path], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_no_secret_and_loopback_only(self):
        os.makedirs(self.state, mode=0o700)
        with open(os.path.join(self.state, "secret"), "w") as f:
            f.write(self.SECRET_TEXT + "\n")
        os.chmod(os.path.join(self.state, "secret"), 0o600)
        out = self.install("--port", "9911")
        self.assertNotIn(self.SECRET_TEXT, out)
        for b in self.plists(out):
            self.assertNotIn("--secret ", b)
            self.assertNotIn("0.0.0.0", b)
        self.assertIn("127.0.0.1", out)
        self.assertNotIn("0.0.0.0", out)
        self.assertIn("9911", out)

    def test_prints_launchctl_commands_and_ps_note(self):
        out = self.install()
        for label in (self.RECEIVER_LABEL, self.FORWARDER_LABEL):
            self.assertIn("launchctl bootstrap gui/$(id -u) ", out)
            self.assertIn("launchctl bootout gui/$(id -u)/" + label, out)
            self.assertIn("launchctl print gui/$(id -u)/" + label, out)
        self.assertIn("visible in ps", out)
        self.assertIn("#1333", out)
        self.assertIn("~/Library/LaunchAgents", out)

    def test_runs_nothing_and_writes_nothing(self):
        self.install()
        self.install("--systemd")
        self.assertFalse(os.path.exists(self.rec), "install-launchd ran gh or launchctl")
        self.assertFalse(os.path.exists(os.path.join(self.t, "Library")))
        self.assertFalse(os.path.exists(self.state), "install-launchd created the state dir")

    def test_systemd_variant(self):
        out = self.install("--systemd")
        self.assertEqual(self.plists(out), [])
        self.assertEqual(out.count("[Unit]"), 2)
        self.assertEqual(out.count("[Service]"), 2)
        self.assertEqual(out.count("Restart=always"), 2)
        self.assertIn("RestartSec=", out)
        self.assertIn("WantedBy=default.target", out)
        self.assertIn("ExecStart=" + os.path.join(HERE, "gh-watch") + " serve", out)
        self.assertIn("ExecStart=" + os.path.join(HERE, "gh-watch-forward"), out)
        self.assertIn("systemctl --user enable --now", out)
        self.assertIn("StandardOutput=append:" + self.state + "/", out)
        self.assertNotIn(self.SECRET_TEXT, out)

    def test_status_reports_launchd_jobs_with_fake_launchctl(self):
        env = dict(self.env)
        with open(os.path.join(self.bin, "launchctl"), "w") as f:
            f.write('#!/bin/sh\necho "launchctl $@" >> "%s"\n'
                    'case "$2" in */%s) exit 0 ;; *) exit 113 ;; esac\n' % (self.rec, self.RECEIVER_LABEL))
        with open(os.path.join(self.bin, "ps"), "w") as f:
            f.write("#!/bin/sh\necho /sbin/launchd\n")
        os.chmod(os.path.join(self.bin, "ps"), 0o755)
        r = subprocess.run([LAUNCHER, "status", "--out-dir", self.state, "--port", "1"], env=env,
                           capture_output=True, text=True, timeout=30)
        self.assertIn("launchd receiver job (" + self.RECEIVER_LABEL + "): loaded", r.stdout)
        self.assertIn("launchd forwarder job (" + self.FORWARDER_LABEL + "): not loaded", r.stdout)
        with open(self.rec) as f:
            calls = f.read().splitlines()
        self.assertEqual(len(calls), 2)
        for c in calls:
            self.assertTrue(c.startswith("launchctl print gui/"), c)
        self.assertFalse(any(c.startswith("gh") for c in calls))

    def test_status_skips_launchd_lines_without_launchctl(self):
        os.remove(os.path.join(self.bin, "launchctl"))
        env = {"PATH": self.bin, "HOME": self.t, "TMPDIR": self.t}
        # a PATH with only the fake bin (ps and friends are symlinked in) so launchctl is truly absent
        for tool in ("ps", "head", "python3", "id", "kill", "dirname", "cat", "env", "bash"):
            import shutil
            src = shutil.which(tool, path="/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin")
            if src and not os.path.exists(os.path.join(self.bin, tool)):
                os.symlink(src, os.path.join(self.bin, tool))
        r = subprocess.run([LAUNCHER, "status", "--out-dir", self.state, "--port", "1"], env=env,
                           capture_output=True, text=True, timeout=30)
        self.assertNotIn("launchd", r.stdout)
        self.assertNotIn("launchctl", r.stderr)

    # --- the forwarder wrapper ---
    FORWARD = os.path.join(HERE, "gh-watch-forward")

    def run_forward(self, *args):
        return subprocess.run([self.FORWARD, "--out-dir", self.state, *args], env=self.env, capture_output=True,
                              text=True, timeout=30)

    def test_wrapper_refuses_missing_secret(self):
        r = self.run_forward()
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(os.path.exists(self.rec), "gh ran with no secret file")

    def test_wrapper_refuses_loose_mode(self):
        os.makedirs(self.state, mode=0o700)
        path = os.path.join(self.state, "secret")
        with open(path, "w") as f:
            f.write(self.SECRET_TEXT + "\n")
        os.chmod(path, 0o644)
        r = self.run_forward()
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("0600", r.stderr)
        self.assertNotIn(self.SECRET_TEXT, r.stdout + r.stderr)
        self.assertFalse(os.path.exists(self.rec), "gh ran with a loose secret file")

    def test_wrapper_execs_gh_webhook_forward_and_never_echoes(self):
        os.makedirs(self.state, mode=0o700)
        path = os.path.join(self.state, "secret")
        with open(path, "w") as f:
            f.write(self.SECRET_TEXT + "\n")
        os.chmod(path, 0o600)
        r = self.run_forward("--port", "9911")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertNotIn(self.SECRET_TEXT, r.stdout + r.stderr)
        with open(self.rec) as f:
            calls = f.read().splitlines()
        self.assertEqual(len(calls), 1, calls)
        self.assertTrue(calls[0].startswith("gh webhook forward "), calls[0])
        self.assertIn("--repo=hslee16/klatch", calls[0])
        self.assertIn("--url=http://127.0.0.1:9911/hook", calls[0])
        self.assertIn("--secret " + self.SECRET_TEXT, calls[0])

    def test_wrapper_source_has_no_other_gh_call(self):
        with open(self.FORWARD, encoding="utf-8") as f:
            src = f.read()
        self.assertNotIn("auth status", src)
        # since #1350 the wrapper runs gh as a supervised child (it reads its stderr); still exactly one gh call
        self.assertNotIn("exec gh ", src)
        gh_lines = [l for l in src.splitlines() if l.strip().startswith("gh ")]
        self.assertEqual(len(gh_lines), 1, gh_lines)
        self.assertTrue(gh_lines[0].strip().startswith("gh webhook forward "), gh_lines)


class StaleHookTests(unittest.TestCase):
    """#1350: a killed forwarder leaves its development webhook; `gh webhook forward` then fails 422 on every restart.
    The wrapper exits 75 with one clear line; `gh-watch clean-hooks` reads the hook list and PRINTS the delete command."""

    FORWARD = os.path.join(HERE, "gh-watch-forward")
    SECRET_TEXT = "STALESECRETVALUE"
    HOOK_URL = "https://webhook-forwarder.github.com/hook"
    LINE = "stale webhook on the repository: run gh-watch clean-hooks"

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.t = self.tmp.name
        self.bin = os.path.join(self.t, "bin")
        os.mkdir(self.bin)
        self.rec = os.path.join(self.t, "ran")
        self.fwd_err = os.path.join(self.t, "fwd.err")
        self.fwd_rc = os.path.join(self.t, "fwd.rc")
        self.hooks = os.path.join(self.t, "hooks.json")
        self.state = os.path.join(self.t, "state")
        os.makedirs(self.state, mode=0o700)
        with open(os.path.join(self.bin, "gh"), "w") as f:
            f.write('''#!/bin/sh
echo "gh $@" >> "%s"
case "$1" in
  webhook) cat "%s" >&2; exit "$(cat "%s")" ;;
  api) case "$*" in *DELETE*) echo deleted; exit 0 ;; *) cat "%s"; exit 0 ;; esac ;;
esac
exit 0
''' % (self.rec, self.fwd_err, self.fwd_rc, self.hooks))
        os.chmod(os.path.join(self.bin, "gh"), 0o755)
        self.set_forward(1, "Error: error creating webhook: HTTP 422: Validation Failed: Hook already exists on this repository\n")
        self.set_hooks([])
        self.env = {"PATH": self.bin + ":/usr/bin:/bin", "HOME": self.t, "TMPDIR": self.t}
        path = os.path.join(self.state, "secret")
        with open(path, "w") as f:
            f.write(self.SECRET_TEXT + "\n")
        os.chmod(path, 0o600)

    def tearDown(self):
        self.tmp.cleanup()

    def set_forward(self, rc, err):
        with open(self.fwd_rc, "w") as f:
            f.write("%d\n" % rc)
        with open(self.fwd_err, "w") as f:
            f.write(err)

    def set_hooks(self, hooks):
        with open(self.hooks, "w") as f:
            json.dump(hooks, f)

    def calls(self):
        try:
            with open(self.rec) as f:
                return f.read().splitlines()
        except OSError:
            return []

    def forward(self):
        return subprocess.run([self.FORWARD, "--out-dir", self.state], env=self.env, capture_output=True, text=True,
                              timeout=30)

    def launcher(self, *args):
        return subprocess.run([LAUNCHER, *args, "--out-dir", self.state], env=self.env, capture_output=True,
                              text=True, timeout=30)

    def hook(self, hid, name="cli", url=None):
        return {"id": hid, "name": name, "active": True, "config": {"url": url or self.HOOK_URL, "content_type": "json"}}

    # --- the wrapper ---
    def test_wrapper_422_hook_exists_exits_75_with_one_line(self):
        r = self.forward()
        self.assertEqual(r.returncode, 75, r.stdout + r.stderr)
        self.assertEqual(r.stderr.count(self.LINE), 1, r.stderr)
        self.assertNotIn(self.SECRET_TEXT, r.stdout + r.stderr)
        self.assertTrue(os.path.exists(os.path.join(self.state, "forwarder.stale")))
        self.assertEqual(len(self.calls()), 1, self.calls())
        self.assertTrue(self.calls()[0].startswith("gh webhook forward "))

    def test_wrapper_other_failure_keeps_its_status_and_clears_the_marker(self):
        with open(os.path.join(self.state, "forwarder.stale"), "w") as f:
            f.write("old\n")
        self.set_forward(1, "Error: something else broke\n")
        r = self.forward()
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("something else broke", r.stderr)
        self.assertNotIn(self.LINE, r.stderr)
        self.assertFalse(os.path.exists(os.path.join(self.state, "forwarder.stale")))

    def test_wrapper_other_status_with_hook_text_is_not_stale(self):
        self.set_forward(2, "Hook already exists\n")
        r = self.forward()
        self.assertEqual(r.returncode, 2)
        self.assertNotIn(self.LINE, r.stderr)

    def test_wrapper_success_clears_marker_and_never_echoes_secret(self):
        with open(os.path.join(self.state, "forwarder.stale"), "w") as f:
            f.write("old\n")
        self.set_forward(0, "")
        r = self.forward()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertFalse(os.path.exists(os.path.join(self.state, "forwarder.stale")))

    def test_wrapper_stderr_secret_is_redacted(self):
        self.set_forward(1, "Error: bad flag --secret %s\n" % self.SECRET_TEXT)
        r = self.forward()
        self.assertEqual(r.returncode, 1)
        self.assertNotIn(self.SECRET_TEXT, r.stdout + r.stderr)

    def test_wrapper_sigterm_reaches_gh_and_exits(self):
        with open(os.path.join(self.bin, "gh"), "w") as f:
            f.write('#!/bin/sh\ntrap \'echo term > "%s"; exit 0\' TERM\necho up > "%s"\nwhile :; do sleep 1; done\n'
                    % (os.path.join(self.t, "got-term"), os.path.join(self.t, "up")))
        p = subprocess.Popen([self.FORWARD, "--out-dir", self.state], env=self.env, stdout=subprocess.DEVNULL,
                             stderr=subprocess.DEVNULL)
        try:
            deadline = time.time() + 20
            while not os.path.exists(os.path.join(self.t, "up")) and time.time() < deadline:
                time.sleep(0.05)
            self.assertTrue(os.path.exists(os.path.join(self.t, "up")), "fake gh did not start")
            p.send_signal(signal.SIGTERM)
            p.wait(timeout=20)
            self.assertTrue(os.path.exists(os.path.join(self.t, "got-term")), "SIGTERM did not reach gh")
        finally:
            if p.poll() is None:
                p.kill()
                p.wait()

    # --- clean-hooks ---
    def test_clean_hooks_lists_stale_cli_hook_and_prints_the_delete_command(self):
        self.set_hooks([self.hook(111, name="web", url="https://example.com/x"), self.hook(222)])
        r = self.launcher("clean-hooks")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("gh api -X DELETE repos/hslee16/klatch/hooks/222", r.stdout)
        self.assertNotIn("hooks/111", r.stdout)
        self.assertEqual(self.calls(), ["gh api repos/hslee16/klatch/hooks"])

    def test_clean_hooks_ignores_a_cli_hook_with_another_url_and_a_web_hook_with_the_forwarding_url(self):
        self.set_hooks([self.hook(5, name="cli", url="https://example.com/hook"), self.hook(6, name="web")])
        r = self.launcher("clean-hooks")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertNotIn("DELETE", r.stdout)
        self.assertIn("no stale", r.stdout)

    def test_clean_hooks_none_says_so(self):
        r = self.launcher("clean-hooks")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("no stale", r.stdout)
        self.assertNotIn("DELETE", r.stdout)

    def test_clean_hooks_runs_no_delete_and_has_no_delete_flag(self):
        self.set_hooks([self.hook(222)])
        r = self.launcher("clean-hooks")
        self.assertEqual(r.returncode, 0)
        self.assertFalse(any("DELETE" in c for c in self.calls()), self.calls())
        r = self.launcher("clean-hooks", "--delete-stale")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(any("DELETE" in c for c in self.calls()), self.calls())

    def test_clean_hooks_gh_failure_is_reported(self):
        with open(os.path.join(self.bin, "gh"), "w") as f:
            f.write('#!/bin/sh\necho "HTTP 403" >&2\nexit 1\n')
        r = self.launcher("clean-hooks")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("admin:repo_hook", r.stdout + r.stderr)

    def test_other_subcommands_still_run_no_gh(self):
        for cmd in ("status", "install-launchd", "forward-command"):
            subprocess.run([LAUNCHER, cmd, "--out-dir", self.state, "--port", "1"] if cmd == "status" else [LAUNCHER, cmd, "--out-dir", self.state],
                           env=self.env, capture_output=True, text=True, timeout=30)
        self.assertEqual(self.calls(), [])


if __name__ == "__main__":
    unittest.main(verbosity=1)
