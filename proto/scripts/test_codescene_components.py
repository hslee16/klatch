#!/usr/bin/env python3
"""Tests for proto/scripts/codescene-components (#1457). Standard library only. No test calls CodeScene or any external
host: `apply` talks to a fake HTTP server on 127.0.0.1 with an ephemeral port, started and stopped by the test."""
import http.server
import importlib.machinery
import importlib.util
import json
import os
import stat
import subprocess
import sys
import tempfile
import threading
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
SCRIPT = os.path.join(HERE, "codescene-components")
TOKEN = "tok-SECRET-0123456789"


def load_module():
    loader = importlib.machinery.SourceFileLoader("codescene_components", SCRIPT)
    spec = importlib.util.spec_from_loader("codescene_components", loader)
    mod = importlib.util.module_from_spec(spec)
    loader.exec_module(mod)
    return mod


def clean_env(**extra):
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_") and k != "CODESCENE_TOKEN"}
    env["GIT_CONFIG_GLOBAL"] = os.devnull
    env["GIT_CONFIG_SYSTEM"] = os.devnull
    env.update(extra)
    return env


def run(*args, env=None, cwd=None):
    return subprocess.run([sys.executable, "-I", SCRIPT, *args], capture_output=True, text=True,
                          env=env if env is not None else clean_env(), cwd=cwd, timeout=60)


def make_repo(files, components):
    """A throwaway git repo (index only) with the given files and a components file; returns (root, components_path)."""
    root = tempfile.mkdtemp(prefix="cs-components-test.")
    subprocess.run(["git", "init", "-q", root], check=True, env=clean_env())
    for rel, body in files.items():
        path = os.path.join(root, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as f:
            f.write(body)
    subprocess.run(["git", "-C", root, "add", "-A"], check=True, env=clean_env())
    cpath = os.path.join(root, "components.json")
    with open(cpath, "w") as f:
        json.dump(components, f)
    return root, cpath


GOOD = [{"name": "a", "patterns": ["src/a/**"]}, {"name": "b", "patterns": ["src/b/**", "tools/*.sh"]}]
GOOD_FILES = {"src/a/x.rs": "", "src/b/y.py": "", "tools/t.sh": "", "README.md": "docs"}


class PartitionCheck(unittest.TestCase):
    def test_real_tree_is_an_exact_partition(self):
        r = run("check", "--root", REPO)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_good_fixture_passes(self):
        root, c = make_repo(GOOD_FILES, GOOD)
        r = run("check", "--root", root, "--components", c)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_file_in_two_components_fails(self):
        comps = GOOD + [{"name": "c", "patterns": ["src/a/x.rs"]}]
        root, c = make_repo(GOOD_FILES, comps)
        r = run("check", "--root", root, "--components", c)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("src/a/x.rs", r.stdout + r.stderr)
        self.assertIn("more than one", r.stdout + r.stderr)

    def test_uncovered_source_file_fails(self):
        root, c = make_repo({**GOOD_FILES, "newdir/z.ts": ""}, GOOD)
        r = run("check", "--root", root, "--components", c)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("newdir/z.ts", r.stdout + r.stderr)

    def test_uncovered_extensionless_script_fails_but_plain_file_is_fine(self):
        root, c = make_repo({**GOOD_FILES, "bin/tool": "#!/usr/bin/env bash\n", "bin/LICENSE": "text"}, GOOD)
        r = run("check", "--root", root, "--components", c)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("bin/tool", r.stdout + r.stderr)
        self.assertNotIn("bin/LICENSE", r.stdout + r.stderr)

    def test_empty_component_fails(self):
        comps = GOOD + [{"name": "empty", "patterns": []}]
        root, c = make_repo(GOOD_FILES, comps)
        r = run("check", "--root", root, "--components", c)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("empty", r.stdout + r.stderr)

    def test_glob_matching_nothing_fails(self):
        comps = [{"name": "a", "patterns": ["src/a/**", "gone/**"]}, GOOD[1]]
        root, c = make_repo(GOOD_FILES, comps)
        r = run("check", "--root", root, "--components", c)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("gone/**", r.stdout + r.stderr)

    def test_glob_semantics(self):
        m = load_module()
        self.assertTrue(m.glob_match("src/a/**", "src/a/b/c.rs"))
        self.assertTrue(m.glob_match("src/*.rs", "src/x.rs"))
        self.assertFalse(m.glob_match("src/*.rs", "src/sub/x.rs"))
        self.assertFalse(m.glob_match("src/a/**", "src/ab/c.rs"))


class Formats(unittest.TestCase):
    def test_csv_has_no_header_two_columns_one_line_per_glob(self):
        root, c = make_repo(GOOD_FILES, GOOD)
        r = run("csv", "--root", root, "--components", c)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.splitlines(), ["a,src/a/**", "b,src/b/**", "b,tools/*.sh"])

    def test_csv_refuses_a_comma_in_a_name_or_glob(self):
        for comps in ([{"name": "a,b", "patterns": ["x/**"]}], [{"name": "ab", "patterns": ["x,y/**"]}]):
            root, c = make_repo(GOOD_FILES, comps)
            r = run("csv", "--root", root, "--components", c)
            self.assertNotEqual(r.returncode, 0)
            self.assertEqual(r.stdout, "")
            self.assertIn("comma", r.stderr)

    def test_json_shape(self):
        root, c = make_repo(GOOD_FILES, GOOD)
        r = run("json", "--root", root, "--components", c)
        self.assertEqual(json.loads(r.stdout), GOOD)

    def test_json_leading_slash(self):
        root, c = make_repo(GOOD_FILES, GOOD)
        r = run("json", "--leading-slash", "--root", root, "--components", c)
        self.assertEqual(json.loads(r.stdout)[1]["patterns"], ["/src/b/**", "/tools/*.sh"])

    def test_real_components_file_loads_and_has_the_planned_areas(self):
        r = run("json", "--root", REPO)
        names = {c["name"] for c in json.loads(r.stdout)}
        for want in ("gateway", "store", "domain", "klatch-server", "klatch-bench", "clients", "coordbench"):
            self.assertIn(want, names)


class FakeCodeScene(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):  # the Authorization header must never be logged, so nothing is
        pass

    def _reply(self, code, body):
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _handle(self):
        srv = self.server
        n = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(n).decode() if n else ""
        srv.requests.append({"method": self.command, "path": self.path, "auth": self.headers.get("Authorization"),
                             "body": body})
        if self.headers.get("Authorization") != "Bearer " + TOKEN:
            return self._reply(401, {"message": "bad token"})
        if srv.force_status.get(self.command):
            return self._reply(srv.force_status[self.command], {"message": "forced"})
        if self.command == "GET":
            return self._reply(200, srv.existing)
        return self._reply(200, {"ok": True})

    do_GET = do_PUT = do_POST = _handle


class Apply(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.srv = http.server.HTTPServer(("127.0.0.1", 0), FakeCodeScene)  # bound and listening on return
        cls.url = "http://127.0.0.1:%d" % cls.srv.server_address[1]
        cls.thread = threading.Thread(target=cls.srv.serve_forever, daemon=True)
        cls.thread.start()
        cls.root, cls.cpath = make_repo(GOOD_FILES, GOOD)

    @classmethod
    def tearDownClass(cls):
        cls.srv.shutdown()
        cls.srv.server_close()
        cls.thread.join(timeout=10)

    def setUp(self):
        self.srv.requests = []
        self.srv.force_status = {}
        self.srv.existing = [{"name": "old", "patterns": ["/old/**"]}]

    def apply(self, *args, token=TOKEN, env_extra=None):
        env = clean_env(**({"CODESCENE_TOKEN": token} if token else {}), **(env_extra or {}))
        return run("apply", "--root", self.root, "--components", self.cpath, "--base-url", self.url, "--project-id", "42",
                   *args, env=env)

    def assert_no_token(self, r):
        self.assertNotIn(TOKEN, r.stdout + r.stderr)
        self.assertNotIn("Bearer " + TOKEN, r.stdout + r.stderr)

    def test_without_yes_it_shows_existing_and_diff_and_does_not_put(self):
        r = self.apply()
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual([q["method"] for q in self.srv.requests], ["GET"])
        self.assertEqual(self.srv.requests[0]["path"], "/v2/projects/42/components")
        out = r.stdout + r.stderr
        self.assertIn("old", out)
        self.assertIn("/old/**", out)
        self.assertIn("--yes", out)
        self.assert_no_token(r)

    def test_with_yes_it_puts_the_payload(self):
        r = self.apply("--yes")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual([q["method"] for q in self.srv.requests], ["GET", "PUT"])
        put = self.srv.requests[1]
        self.assertEqual(put["path"], "/v2/projects/42/components")
        self.assertEqual(put["auth"], "Bearer " + TOKEN)
        self.assertEqual(json.loads(put["body"]), GOOD)
        self.assert_no_token(r)

    def test_leading_slash_flag_changes_the_body(self):
        r = self.apply("--yes", "--leading-slash")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(json.loads(self.srv.requests[1]["body"])[0]["patterns"], ["/src/a/**"])

    def test_run_analysis_posts_after_the_put(self):
        r = self.apply("--yes", "--run-analysis")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual([q["method"] for q in self.srv.requests], ["GET", "PUT", "POST"])
        self.assertEqual(self.srv.requests[2]["path"], "/v2/projects/42/run-analysis")

    def test_dry_run_sends_nothing_and_redacts_the_token(self):
        r = self.apply("--dry-run", "--yes")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(self.srv.requests, [])
        self.assertIn("PUT", r.stdout)
        self.assertIn("redacted", r.stdout)
        self.assert_no_token(r)

    def test_dry_run_needs_no_token(self):
        r = self.apply("--dry-run", token=None)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(self.srv.requests, [])

    def test_token_from_a_0600_file_works(self):
        d = tempfile.mkdtemp(prefix="cs-token.")
        p = os.path.join(d, "token")
        with open(p, "w") as f:
            f.write(TOKEN + "\n")
        os.chmod(p, 0o600)
        r = self.apply("--yes", "--token-file", p, token=None)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(self.srv.requests[1]["auth"], "Bearer " + TOKEN)
        self.assert_no_token(r)

    def test_token_from_a_loose_file_is_refused(self):
        d = tempfile.mkdtemp(prefix="cs-token.")
        for mode in (0o640, 0o604, 0o644):
            p = os.path.join(d, "token%o" % mode)
            with open(p, "w") as f:
                f.write(TOKEN + "\n")
            os.chmod(p, mode)
            r = self.apply("--yes", "--token-file", p, token=None)
            self.assertNotEqual(r.returncode, 0)
            self.assertEqual(self.srv.requests, [])
            self.assertIn("0600", r.stderr)
            self.assert_no_token(r)

    def test_no_token_is_an_error(self):
        r = self.apply("--yes", token=None)
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual(self.srv.requests, [])
        self.assertIn("CODESCENE_TOKEN", r.stderr)

    def test_there_is_no_token_argument(self):
        r = self.apply("--yes", "--token", TOKEN, token=None)
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual(self.srv.requests, [])

    def test_plain_http_to_a_non_loopback_host_is_refused(self):
        env = clean_env(CODESCENE_TOKEN=TOKEN)
        r = run("apply", "--root", self.root, "--components", self.cpath, "--base-url", "http://example.invalid",
                "--project-id", "42", "--yes", env=env)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("https", r.stderr)

    def test_error_statuses_have_clear_messages(self):
        for code, word in ((401, "token"), (403, "permission"), (404, "project"), (422, "rejected")):
            self.srv.requests = []
            self.srv.force_status = {"GET": code}
            r = self.apply("--yes")
            self.assertNotEqual(r.returncode, 0, code)
            self.assertIn(word, (r.stdout + r.stderr).lower(), code)
            self.assertIn(str(code), r.stdout + r.stderr)
            self.assert_no_token(r)
            self.assertEqual([q["method"] for q in self.srv.requests], ["GET"])

    def test_put_error_is_reported(self):
        self.srv.force_status = {"PUT": 422}
        r = self.apply("--yes")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("422", r.stdout + r.stderr)
        self.assertIn("rejected", (r.stdout + r.stderr).lower())

    def test_wrong_token_gives_401_message(self):
        r = self.apply("--yes", token="wrong-token-value")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("401", r.stdout + r.stderr)
        self.assertNotIn("wrong-token-value", r.stdout + r.stderr)


if __name__ == "__main__":
    unittest.main()
