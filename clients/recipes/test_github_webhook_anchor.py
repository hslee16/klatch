"""Tests for github-webhook-anchor.py (#531): the signature check in front of `klatch anchor github`.

Run from the repository root: `python3 -m unittest discover -s clients/recipes -p 'test_*.py'`.

No network, no real secret: the secret is a made-up test value, the payloads are fixtures, and the klatch the script
calls is a stand-in that records what it was given. One test runs the real binary when one is built.
"""
import hashlib
import hmac
import importlib.util
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "github-webhook-anchor.py")
FIXTURES = os.path.join(HERE, "fixtures", "github")
SECRET = b"test-secret-not-a-real-one"


def load():
    spec = importlib.util.spec_from_file_location("github_webhook_anchor", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def fixture(name):
    with open(os.path.join(FIXTURES, name), "rb") as f:
        return f.read()


def sign(body, secret=SECRET):
    return "sha256=" + hmac.new(secret, body, hashlib.sha256).hexdigest()


class Verify(unittest.TestCase):
    def test_the_published_test_vector_verifies(self):
        # the example in GitHub's webhook documentation: this secret and this body
        header = "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17"
        self.assertTrue(load().verify(b"It's a Secret to Everybody", b"Hello, World!", header))

    def test_only_a_well_formed_matching_sha256_header_verifies(self):
        verify, body = load().verify, b"{}"
        good = sign(body)
        self.assertTrue(verify(SECRET, body, good))
        for bad in (
            None,
            "",
            good[len("sha256="):],  # no prefix
            "sha1=" + hmac.new(SECRET, body, hashlib.sha1).hexdigest(),  # the old header's algorithm
            good.upper(),
            good[:-2],  # short
            good + "00",  # long
            "sha256=" + "g" * 64,
            sign(body, b"another secret"),
            sign(b"{ }"),
        ):
            self.assertFalse(verify(SECRET, body, bad), repr(bad))

    def test_an_empty_secret_never_verifies(self):
        self.assertFalse(load().verify(b"", b"{}", sign(b"{}", b"")))


class Script(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="klatch-hook-")
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.secret_file = self.path("secret")
        with open(self.secret_file, "wb") as f:
            f.write(SECRET + b"\n")  # a trailing newline, as an editor leaves it
        self.record = self.path("record")
        self.fake = self.path("klatch")
        self.fake_exit = 0
        self.write_fake()

    def path(self, name):
        return os.path.join(self.dir, name)

    def write_fake(self):
        with open(self.fake, "w") as f:
            f.write(
                "#!/bin/sh\n"
                f'echo "$@" > "{self.record}.args"\n'
                f'cat > "{self.record}.stdin"\n'
                'echo "released 1 claim(s)"\n'
                f"exit {self.fake_exit}\n"
            )
        os.chmod(self.fake, os.stat(self.fake).st_mode | stat.S_IXUSR)

    def run_script(self, body, signature="sign", event="pull_request", extra=()):
        args = [sys.executable, "-I", SCRIPT, "--secret-file", self.secret_file, "--klatch", self.fake, *extra]
        if event is not None:
            args += ["--event", event]
        if signature == "sign":
            signature = sign(body)
        if signature is not None:
            args += ["--signature", signature]
        return subprocess.run(args, input=body, capture_output=True, timeout=30)

    def forwarded(self):
        return os.path.exists(self.record + ".stdin")

    def test_a_signed_pull_request_is_piped_to_klatch_anchor_github_unchanged(self):
        body = fixture("pull_request_closed.json")
        r = self.run_script(body)
        self.assertEqual(r.returncode, 0, r.stderr)
        with open(self.record + ".args") as f:
            self.assertEqual(f.read().split(), ["anchor", "github"])
        with open(self.record + ".stdin", "rb") as f:
            self.assertEqual(f.read(), body)
        self.assertIn(b"released 1 claim(s)", r.stdout)

    def test_a_signed_issue_event_is_forwarded_too(self):
        r = self.run_script(fixture("issues_closed.json"), event="issues")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertTrue(self.forwarded())

    def test_a_wrong_or_missing_signature_forwards_nothing_and_exits_3(self):
        body = fixture("pull_request_closed.json")
        for signature in (sign(body, b"another secret"), sign(b"tampered"), "sha256=00", None):
            r = self.run_script(body, signature=signature)
            self.assertEqual(r.returncode, 3, (signature, r.stderr))
            self.assertFalse(self.forwarded(), signature)

    def test_a_refusal_never_prints_the_secret_the_body_or_the_expected_signature(self):
        body = fixture("pull_request_closed.json")
        r = self.run_script(body, signature=sign(b"tampered"))
        shown = r.stdout + r.stderr
        for private in (SECRET, b"acme/widgets", sign(body).encode(), sign(b"tampered").encode()):
            self.assertNotIn(private, shown)

    def test_a_signed_ping_and_an_event_klatch_does_not_read_are_acknowledged_and_not_forwarded(self):
        for event, body in (("ping", fixture("ping.json")), ("star", b'{"action":"created"}')):
            r = self.run_script(body, event=event)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertFalse(self.forwarded(), event)
            self.assertIn(b"ignored", r.stderr)

    def test_an_unsigned_ping_is_still_refused(self):
        r = self.run_script(fixture("ping.json"), event="ping", signature=None)
        self.assertEqual(r.returncode, 3)

    def test_the_signature_is_checked_before_the_event_is_looked_at(self):
        r = self.run_script(b"{}", event="star", signature="sha256=00")
        self.assertEqual(r.returncode, 3)

    def test_a_body_over_the_limit_is_refused_before_anything_else(self):
        body = fixture("pull_request_closed.json")
        r = self.run_script(body, extra=("--max-bytes", "10"))
        self.assertEqual(r.returncode, 4, r.stderr)
        self.assertFalse(self.forwarded())

    def test_a_missing_or_empty_secret_is_a_configuration_error(self):
        body = fixture("pull_request_closed.json")
        for contents in (None, b"", b"\n"):
            if contents is None:
                os.remove(self.secret_file)
            else:
                with open(self.secret_file, "wb") as f:
                    f.write(contents)
            r = self.run_script(body)
            self.assertEqual(r.returncode, 2, (contents, r.stderr))
            self.assertFalse(self.forwarded())

    def test_a_missing_event_header_is_a_configuration_error_not_a_forward(self):
        r = self.run_script(fixture("pull_request_closed.json"), event=None)
        self.assertEqual(r.returncode, 2)
        self.assertFalse(self.forwarded())

    def test_klatchs_own_failure_is_passed_on(self):
        self.fake_exit = 5
        self.write_fake()
        r = self.run_script(fixture("pull_request_closed.json"))
        self.assertEqual(r.returncode, 5)

    def test_a_klatch_that_cannot_be_started_is_an_error_after_a_good_signature(self):
        body = fixture("pull_request_closed.json")
        r = self.run_script(body, extra=("--klatch", self.path("no-such-klatch")))
        self.assertEqual(r.returncode, 2, r.stderr)

    def test_the_secret_is_read_from_a_file_and_never_from_the_command_line(self):
        r = subprocess.run([sys.executable, "-I", SCRIPT, "--help"], capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 0)
        self.assertIn("--secret-file", r.stdout)
        self.assertNotRegex(r.stdout, r"--secret\b(?!-file)")

    def test_the_fixtures_are_the_payloads_klatch_anchor_github_reads(self):
        for name in ("pull_request_closed.json", "issues_closed.json", "ping.json"):
            json.loads(fixture(name))


BIN = os.environ.get("KLATCH_BIN") or os.path.join(HERE, "..", "..", "proto", "target", "debug", "klatch")


@unittest.skipUnless(os.access(BIN, os.X_OK), "no klatch binary built")
class WithTheRealBinary(unittest.TestCase):
    def test_a_signed_merge_cannot_release_the_claim_unattended_since_the_admin_passphrase(self):
        home = tempfile.mkdtemp(prefix="klatch-hook-real-")
        self.addCleanup(shutil.rmtree, home, True)
        fake_home = os.path.join(home, "home")
        os.mkdir(fake_home)
        env = {
            "PATH": os.environ["PATH"],
            "HOME": fake_home,
            "KLATCH_HOME": os.path.join(home, "store"),
            "KLATCH_KEY_DIR": os.path.join(home, "keys"),
        }
        member = dict(env, KLATCH_NAME="worker")
        claim = subprocess.run(
            [BIN, "claim", "r", "src/a.rs", "--anchor", "gh:pr:acme/widgets/7"], env=member, capture_output=True, text=True
        )
        self.assertEqual(claim.returncode, 0, claim.stderr)
        secret = os.path.join(home, "secret")
        with open(secret, "wb") as f:
            f.write(SECRET)
        body = fixture("pull_request_closed.json")
        r = subprocess.run(
            [sys.executable, "-I", SCRIPT, "--secret-file", secret, "--klatch", BIN, "--event", "pull_request",
             "--signature", sign(body)],
            input=body, env=env, capture_output=True, timeout=60,
        )
        # `klatch anchor github` is a launcher command and asks for the admin passphrase at a terminal (#1233); a
        # webhook handler has none, so it is refused and nothing is released (the lease still ends the claim). Ticketed.
        self.assertNotEqual(r.returncode, 0)
        self.assertIn(b"admin passphrase", r.stderr)
        claims = subprocess.run([BIN, "claims", "r"], env=member, capture_output=True, text=True)
        self.assertIn("src/a.rs", claims.stdout)


if __name__ == "__main__":
    unittest.main()
