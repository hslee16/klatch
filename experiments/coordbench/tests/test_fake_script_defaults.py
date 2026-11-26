"""A fixture that names a fake script has that file (#1511); a fake run of one that does not is refused, not crashed. A
`send` step sends its text as written, with no {{ROOM}} or {{NAME}} filled in (#1512)."""
import glob
import json
import os
import subprocess
import sys
import tempfile
import unittest

from helpers import BENCH
from test_run_fake import Base


class TestFakeScriptNamed(unittest.TestCase):
    def test_every_fixture_that_names_a_fake_script_has_the_file(self):
        for path in glob.glob(os.path.join(BENCH, "fixtures", "*", "fixture.json")):
            with open(path) as f:
                spec = json.load(f)
            if spec.get("fake"):
                self.assertTrue(os.path.exists(os.path.join(os.path.dirname(path), spec["fake"])), path)


class TestFakeRunWithoutScript(Base):
    def test_a_fake_run_of_a_fixture_without_a_fake_script_is_refused(self):
        argv = self.argv("B1", fixture="specialists")
        i = argv.index("--fixture")
        argv[i + 1] = "specialists"
        code, _, err = self.go(argv)
        self.assertEqual(code, 2)
        self.assertIn("the fixture has no fake script", err)


class TestSendIsNotFilled(unittest.TestCase):
    def test_a_send_text_keeps_its_placeholders(self):
        with tempfile.TemporaryDirectory() as d:
            script = os.path.join(d, "s.json")
            with open(script, "w") as f:
                json.dump({"agents": {}, "default": [{"op": "send", "to": "b", "text": "in {{ROOM}} as {{NAME}}"}]}, f)
            out = subprocess.run([sys.executable, os.path.join(BENCH, "fake_agent.py"), "--script", script, "--name", "a",
                                  "--arm", "B1", "--out", d, "--room", "lobby"], capture_output=True, text=True, cwd=d).stdout
        msgs = [b["input"]["message"] for line in out.splitlines() if line.startswith("{")
                for b in json.loads(line).get("message", {}).get("content", [])
                if isinstance(b, dict) and b.get("name") == "SendMessage"]
        self.assertEqual(msgs, ["in {{ROOM}} as {{NAME}}"])


if __name__ == "__main__":
    unittest.main()
