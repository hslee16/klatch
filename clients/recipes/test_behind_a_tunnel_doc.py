"""Checks on clients/recipes/behind-a-tunnel.md (#110, epic #13): the deployment guide for reaching a loopback-only
`klatch-server http` from another machine through a tunnel or a TLS terminator on the same host.

Rules the page must keep, each checked here:
- klatch stays loopback only: no `klatch-server` command line in the page listens on anything but 127.0.0.1 or [::1];
- the page says identity is the worker credential and that forwarded headers are ignored, and names the prerequisites
  (#157, #161, #108) that are still open;
- the plaintext hop is allowed only on the same host, and the page says a hop between hosts is never allowed;
- nothing on the page was run against a real ssh, sshd or stunnel, and it says `not run` for each;
- every repository file the page points at exists.
Run from the repository root: `python3 -m unittest discover -s clients/recipes -p 'test_*.py'`.
"""
import os
import re
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
PAGE = os.path.join(HERE, "behind-a-tunnel.md")


def page():
    with open(PAGE, encoding="utf-8") as f:
        return f.read()


class BehindATunnelDoc(unittest.TestCase):
    def test_the_page_exists(self):
        self.assertTrue(os.path.exists(PAGE))

    def test_every_klatch_server_listen_address_is_loopback(self):
        text = page()
        listens = re.findall(r"klatch-server\s+http[^\n`]*--listen\s+(\S+)", text)
        self.assertTrue(listens, "the page shows how to start the listener")
        for addr in listens:
            self.assertRegex(addr, r"^(127\.0\.0\.1|\[::1\]):", addr)

    def test_identity_is_the_credential_and_forwarded_headers_are_ignored(self):
        text = page()
        self.assertIn("X-Forwarded-For", text)
        self.assertRegex(text, r"(?i)ignored")
        self.assertRegex(text, r"(?i)worker credential")
        self.assertIn("X-Klatch-Member", text)

    def test_the_open_prerequisites_are_named(self):
        text = page()
        for ticket in ("#157", "#161", "#108", "#110"):
            self.assertIn(ticket, text)

    def test_a_plaintext_hop_is_allowed_on_one_host_only(self):
        text = page()
        self.assertRegex(text, r"(?i)same host")
        self.assertRegex(text, r"(?i)plaintext hop between hosts is refused|never a plaintext hop between hosts")

    def test_nothing_was_run_and_the_page_says_so(self):
        text = page()
        for tool in ("ssh", "stunnel"):
            self.assertRegex(text, rf"(?i){tool}[^\n]*not run|not run[^\n]*{tool}")

    def test_the_files_it_points_at_exist(self):
        text = page()
        for target in re.findall(r"\]\(([^)#]+)(?:#[^)]*)?\)", text):
            if target.startswith("http"):
                continue
            path = os.path.normpath(os.path.join(HERE, target))
            self.assertTrue(os.path.exists(path), target)

    def test_no_em_dash(self):
        self.assertNotIn("—", page())


if __name__ == "__main__":
    unittest.main()
