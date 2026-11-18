"""Checks on clients/recipes/ORCHESTRATORS.md (#126): the page may only say what can be verified, and must say so.

It names a set of third-party tools. Rules the page must keep, each checked here:
- every tool has its own section with a `Status:` line that says `run` or `not run`; none of these tools has been run
  against klatch, so every status must say `not run` (change this test only together with a recorded run);
- no version number, flag or option of a third-party tool appears (a flag in a code span, or a `1.2.3` style number),
  because none was checked;
- every repository file the page points at exists;
- the overlap table has one row per tool, and the page ends the claims about a tool with where the claim comes from.
Run from the repository root: `python3 -m unittest discover -s clients/recipes -p 'test_*.py'`.
"""
import os
import re
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
PAGE = os.path.join(HERE, "ORCHESTRATORS.md")

TOOLS = [
    "Claude Code agent teams",
    "Orchestrator.inc",
    "Claude Squad",
    "Conductor",
    "Agent Swarm",
    "General agent frameworks",
]


def page():
    with open(PAGE, encoding="utf-8") as f:
        return f.read()


def sections():
    out, name = {}, None
    for line in page().splitlines():
        if line.startswith("## "):
            name = line[3:].strip()
            out[name] = []
        elif name is not None:
            out[name].append(line)
    return {k: "\n".join(v) for k, v in out.items()}


class OrchestratorsPage(unittest.TestCase):
    def test_every_tool_has_a_section_with_a_status_line(self):
        found = sections()
        for tool in TOOLS:
            self.assertIn(tool, found, f"no section for {tool}")
            self.assertRegex(found[tool], r"(?m)^Status: (run|not run)\b", tool)

    def test_no_tool_is_claimed_to_have_been_run_without_a_recorded_run(self):
        for tool in TOOLS:
            self.assertRegex(sections()[tool], r"(?m)^Status: not run\b", tool)

    def test_no_third_party_version_or_flag_is_stated(self):
        text = page()
        # a section number of this repository's own documents and a licence name are not versions of a tool
        bare = re.sub(r"section \d+(\.\d+)?", "", text).replace("AGPL-3.0", "")
        self.assertIsNone(re.search(r"\bv?\d+\.\d+(\.\d+)?\b", bare), "a version-like number")
        self.assertIsNone(re.search(r"`--[a-z][a-z-]*`", text.replace("`--member-scope`", "").replace("`--shared`", "")
                                    .replace("`--anchor`", "")), "a flag in a code span")

    def test_every_repository_file_it_points_at_exists(self):
        paths = set(re.findall(r"`((?:[A-Za-z_./-]+/)?[A-Za-z_.-]+\.(?:md|sh|py))`", page()))
        self.assertGreater(len(paths), 3)
        for p in paths:
            ok = os.path.exists(os.path.join(ROOT, p)) or os.path.exists(os.path.join(HERE, p))
            self.assertTrue(ok, f"{p} does not exist")

    def test_the_overlap_table_has_a_row_for_every_tool(self):
        table = [l for l in page().splitlines() if l.startswith("|")]
        for tool in TOOLS:
            self.assertTrue(any(l.startswith(f"| {tool} ") for l in table), tool)

    def test_each_tool_section_says_where_its_claims_come_from(self):
        for tool in TOOLS:
            self.assertRegex(sections()[tool], r"(?m)^Source: ", tool)

    def test_the_page_says_how_to_verify_a_tool_and_what_to_report(self):
        text = page()
        self.assertIn("How to verify", text)
        self.assertIn("#414", text)


if __name__ == "__main__":
    unittest.main()
