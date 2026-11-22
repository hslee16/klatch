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
            call = f.read().splitlines()[0].split("|")
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

    def test_log_is_flushed_per_line(self):
        _, port = self.start()
        self.post(port, "push", push())
        # read while the receiver is still running: the line must already be on disk
        self.assertEqual(len(self.log()), 1)
        self.assertTrue(stat.S_IMODE(os.stat(self.out).st_mode) & 0o077 == 0, "out dir should be private")


if __name__ == "__main__":
    unittest.main(verbosity=1)
