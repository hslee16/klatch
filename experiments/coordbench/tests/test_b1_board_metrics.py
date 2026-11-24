"""The board under merge=union and the messages metric (#1520 review, items 3 and 4). With the union attribute a status change
leaves the old `working` line beside the new one, so the claim-conflict metric must read the LAST line per (agent, title); and a
board edit is a message only when it adds a line that starts with one of the shared message prefixes, and not twice when the same
text was just sent with SendMessage. Real git for the boards, recorded-style streams for the counts."""
import os
import shutil
import subprocess
import tempfile
import unittest

from helpers import BENCH  # noqa: F401
import board
import streams
from test_streams import asst, res, tool

KB = "/opt/klatch/klatch"
GENV = {"PATH": os.environ.get("PATH", ""), "HOME": "/nonexistent", "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
        "GIT_TERMINAL_PROMPT": "0", "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t.invalid", "GIT_COMMITTER_NAME": "t",
        "GIT_COMMITTER_EMAIL": "t@t.invalid"}
HEAD = "# Task board\n\n"


class LastLineWins(unittest.TestCase):
    def found(self, body):
        return board.conflicts([HEAD + body])

    def test_a_later_done_line_supersedes_an_earlier_working_line(self):
        self.assertEqual(self.found("- a | working | x.py | T1\n- a | done | x.py | T1\n- b | working | x.py | T2\n"), [])

    def test_two_edits_of_one_line_keep_both_and_the_last_wins(self):
        self.assertEqual(self.found("- a | done | x.py | T1\n- a | blocked | x.py | T1\n- b | working | x.py | T2\n"), [])

    def test_a_real_overlap_is_still_a_conflict(self):
        self.assertEqual(self.found("- a | working | x.py | T1\n- b | working | x.py | T2\n"),
                         [{"file": "x.py", "agents": ["a", "b"]}])

    def test_a_claim_taken_again_after_done_counts_again(self):
        body = "- a | working | x.py | T1\n- a | done | x.py | T1\n- a | working | x.py | T1\n- b | working | x.py | T2\n"
        self.assertEqual(self.found(body), [{"file": "x.py", "agents": ["a", "b"]}])

    def test_the_same_agent_with_two_titles_on_one_file_is_two_independent_lines(self):
        self.assertEqual(self.found("- a | working | x.py | T1\n- a | done | x.py | T2\n- b | working | x.py | T3\n"),
                         [{"file": "x.py", "agents": ["a", "b"]}])

    def test_released_lines_do_not_count_whatever_the_status_word(self):
        for word in ("done", "blocked", "handoff", "Done"):
            self.assertEqual(self.found("- a | working | x.py | T1\n- a | %s | x.py | T1\n- b | working | x.py | T2\n" % word), [], word)


class RealGit(unittest.TestCase):
    def setUp(self):
        self.tmp = os.path.realpath(tempfile.mkdtemp(prefix="coordbench-union-"))
        self.addCleanup(lambda: shutil.rmtree(self.tmp, ignore_errors=True))

    def git(self, cwd, *args):
        p = subprocess.run(["git"] + list(args), cwd=cwd, env=GENV, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        return p.returncode, p.stdout

    def test_a_status_change_merged_by_union_with_another_agents_append_is_no_claim_conflict(self):
        seed = os.path.join(self.tmp, "seed")
        os.makedirs(seed)
        with open(os.path.join(seed, "TASKS.md"), "w") as f:
            f.write(HEAD + "- a | working | x.py | T1\n")
        with open(os.path.join(seed, ".gitattributes"), "w") as f:
            f.write("TASKS.md merge=union\n")
        self.git(seed, "init", "-q", "-b", "main")
        self.git(seed, "add", "-A")
        self.git(seed, "commit", "-q", "-m", "initial")
        origin = os.path.join(self.tmp, "origin.git")
        self.git(self.tmp, "clone", "-q", "--bare", seed, origin)
        ca, cb = os.path.join(self.tmp, "a"), os.path.join(self.tmp, "b")
        for c in (ca, cb):
            self.git(self.tmp, "clone", "-q", origin, c)
        with open(os.path.join(ca, "TASKS.md")) as f:
            text = f.read()
        with open(os.path.join(ca, "TASKS.md"), "w") as f:
            f.write(text.replace("| working |", "| done |"))
        self.git(ca, "commit", "-q", "-am", "a done")
        self.git(ca, "push", "-q", "origin", "main")
        with open(os.path.join(cb, "TASKS.md"), "a") as f:
            f.write("- b | working | x.py | T2\n")
        self.git(cb, "commit", "-q", "-am", "b working")
        code, out = self.git(cb, "pull", "-q", "--no-rebase", "origin", "main")
        self.assertEqual(code, 0, out)
        with open(os.path.join(cb, "TASKS.md")) as f:
            merged = f.read()
        self.assertIn("- b | working | x.py | T2", merged)
        self.assertIn("- a | done | x.py | T1", merged)
        self.assertEqual(board.conflicts([merged]), [], merged)  # the merged version keeps a's old working line before a's done line
        # b's own commit before the pull (a's old working line and b's working line, a's done not yet seen) is a version too and is
        # the one real "blind" overlap board.py has always counted; the merged version adds none
        res = board.claim_conflicts([origin, cb], GENV)
        self.assertEqual(res["claim_conflicts"], 1, res)


def edit(tid, new, old="", path="/w/TASKS.md"):
    return asst("m" + tid, [tool(tid, "Edit", file_path=path, old_string=old, new_string=new)])


def count(events, times=None):
    st = streams.StreamState(klatch_bin=KB)
    for i, ev in enumerate(events):
        st.feed(ev, None if times is None else times[i])
    return st.summary()


class Messages(unittest.TestCase):
    def test_a_claim_or_status_line_is_a_board_edit_and_not_a_message(self):
        s = count([edit("t1", "- a | working | x.py | T1"), edit("t2", "- a | done | x.py | T1", old="- a | working | x.py | T1")])
        self.assertEqual((s["board_edits"], s["messages"]), (2, 0))

    def test_a_handoff_status_line_is_not_a_message_but_a_handoff_prefix_line_is(self):
        self.assertEqual(count([edit("t1", "- a | handoff | x.py | T1")])["messages"], 0)
        self.assertEqual(count([edit("t1", "handoff: T1 to b")])["messages"], 1)

    def test_lines_that_start_with_the_shared_prefixes_are_messages(self):
        for line in ("seeking: what is LEDGER_MODULUS?", "answer: LEDGER_MODULUS=97 (from b)", "handoff: T1", "need-info: x", "info: y"):
            self.assertEqual(count([edit("t1", line)])["messages"], 1, line)

    def test_only_the_added_lines_of_an_edit_count(self):
        old = "answer: A=1 (from b)"
        s = count([edit("t1", old + "\n- a | working | x.py | T1", old=old)])
        self.assertEqual(s["messages"], 0)
        s = count([edit("t1", old + "\nanswer: B=2 (from c)", old=old)])
        self.assertEqual(s["messages"], 1)

    def test_a_shell_append_of_an_answer_line_is_a_message(self):
        s = count([asst("m1", [tool("t1", "Bash", command="echo 'answer: X=1 (from b)' >> TASKS.md")])])
        self.assertEqual(s["messages"], 1)
        s = count([asst("m1", [tool("t1", "Bash", command="echo '- a | working | f.py | T' >> TASKS.md")])])
        self.assertEqual((s["board_edits"], s["messages"]), (1, 0))

    def test_a_write_of_the_board_counts_its_message_lines(self):
        s = count([asst("m1", [tool("t1", "Write", file_path="TASKS.md", content="# t\n- a | working | f | T\nanswer: X=1 (from b)\n")])])
        self.assertEqual(s["messages"], 1)

    def test_a_board_answer_that_repeats_a_send_message_within_30_s_counts_once(self):
        send = asst("m1", [tool("t1", "SendMessage", to="a", message="answer: LEDGER_MODULUS=97")])
        board_ = edit("t2", "answer: LEDGER_MODULUS=97 (from b)")
        self.assertEqual(count([send, board_], [10.0, 15.0])["messages"], 1)
        self.assertEqual(count([board_, send], [10.0, 15.0])["messages"], 1)
        self.assertEqual(count([send, board_], [10.0, 41.0])["messages"], 2)  # not "just" sent
        other = edit("t2", "answer: OTHER=1 (from b)")
        self.assertEqual(count([send, other], [10.0, 15.0])["messages"], 2)

    def test_the_equivalent_k_and_b1_answer_sequences_count_the_same_number_of_messages(self):
        k = count([asst("m1", [tool("t1", "Bash", command=KB + " answer lobby 1 'LEDGER_MODULUS=97'")]), res("t1", "ok")])
        b = count([asst("m1", [tool("t1", "SendMessage", to="a", message="answer: LEDGER_MODULUS=97")]), res("t1", "{}"),
                   edit("t2", "answer: LEDGER_MODULUS=97 (from b)")], [1.0, 1.5, 3.0])
        self.assertEqual(k["messages"], 1)
        self.assertEqual(b["messages"], 1)

    def test_the_real_pilot_run_has_no_messages(self):
        here = os.path.dirname(os.path.abspath(__file__))
        for n in ("a", "b"):
            with open(os.path.join(here, "fixtures", "pilot-b1", n + ".stream.jsonl")) as f:
                st = streams.parse_stream(f.read().splitlines())
            self.assertGreater(st["board_edits"], 0, n)
            self.assertEqual(st["messages"], 0, n)


if __name__ == "__main__":
    unittest.main()
