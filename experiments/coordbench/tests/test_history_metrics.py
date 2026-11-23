"""The S9 stream analysis and the commit replay (#1374, history.py): what counts as a history read, the briefing, the first
correct contribution (found by replaying the agent's commits against the checker), tokens and turns up to it."""
import json
import os
import shutil
import subprocess
import tempfile
import unittest

from helpers import BENCH  # noqa: F401
import history
from test_history_fixture import FIXTURE, GENV, build, git

CHECK = os.path.join(BENCH, "checks", "history", "check.sh")
BIN = "/opt/klatch"


def ev(**kw):
    return json.dumps(kw)


class Stream:
    """A fake agent's stream, built call by call: each call is one assistant message with one tool_use."""

    def __init__(self):
        self.lines = [ev(type="system", subtype="init", model="claude-fake")]
        self.n = 0

    def hook(self, text):
        self.lines.append(ev(type="system", subtype="hook_response", hook_event="SessionStart", output=text))

    def call(self, name, inp, result="", usage=None):
        self.n += 1
        usage = usage or {"input_tokens": 10, "output_tokens": 5}
        self.lines.append(ev(type="assistant", message={"id": "m%d" % self.n, "model": "claude-fake", "usage": usage,
                                                         "content": [{"type": "tool_use", "id": "t%d" % self.n, "name": name,
                                                                      "input": inp}]}))
        self.lines.append(ev(type="user", message={"content": [{"type": "tool_result", "tool_use_id": "t%d" % self.n,
                                                                 "content": result}]}))
        return self

    def bash(self, cmd, result=""):
        return self.call("Bash", {"command": cmd}, result)


class TestClassify(unittest.TestCase):
    def kind(self, name, inp, bin=BIN):
        return history.classify(name, inp, bin)

    def test_git_history_reads(self):
        for cmd in ("git log", "git log --oneline -n 30", "git -C /x log -p", "git show HEAD~3", "git blame parcelkit/rates.py",
                    "cd repo && git log --format=%B", "git shortlog -sn"):
            self.assertEqual(self.kind("Bash", {"command": cmd}), "git_log", cmd)

    def test_other_git_commands_are_not_reads(self):
        for cmd in ("git status", "git add -A", "git commit -m 'log the thing'", "git push origin HEAD:main",
                    "git diff", "python3 -m unittest"):
            self.assertIsNone(self.kind("Bash", {"command": cmd}), cmd)

    def test_the_board_files_by_tool_or_shell(self):
        self.assertEqual(self.kind("Read", {"file_path": "/w/TASKS.md"}), "board_file")
        self.assertEqual(self.kind("Read", {"file_path": "/w/DECISIONS.md"}), "board_file")
        self.assertEqual(self.kind("Bash", {"command": "cat DECISIONS.md"}), "board_file")
        self.assertEqual(self.kind("Bash", {"command": "grep -n tare TASKS.md"}), "board_file")
        self.assertIsNone(self.kind("Read", {"file_path": "/w/README.md"}))
        self.assertIsNone(self.kind("Bash", {"command": "git add TASKS.md"}))

    def test_klatch_reads_by_verb(self):
        for verb in ("history", "read", "board", "find", "asks", "tasks", "claims"):
            self.assertEqual(self.kind("Bash", {"command": "%s %s lobby" % (BIN, verb)}), "klatch_" + verb, verb)
        self.assertEqual(self.kind("Bash", {"command": "%s --discover history lobby --last 50" % BIN}), "klatch_history")
        self.assertEqual(self.kind("Bash", {"command": "KLATCH_NAME=dana %s find tare" % BIN}), "klatch_find")

    def test_klatch_writes_are_not_reads(self):
        for cmd in ("%s post lobby 'working: x'" % BIN, "%s claim lobby f.py" % BIN, "%s join lobby" % BIN,
                    "%s wait lobby" % BIN):
            self.assertIsNone(self.kind("Bash", {"command": cmd}), cmd)

    def test_edits_and_commits(self):
        self.assertTrue(history.is_edit("Write", {}))
        self.assertTrue(history.is_edit("Edit", {}))
        self.assertTrue(history.is_edit("Bash", {"command": "git commit -m x"}))
        self.assertFalse(history.is_edit("Bash", {"command": "git log"}))
        self.assertTrue(history.is_commit("Bash", {"command": "git commit -q -m x"}))
        self.assertFalse(history.is_commit("Bash", {"command": "git add -A"}))


class TestAnalyze(unittest.TestCase):
    def test_reads_by_kind_before_the_first_edit_and_totals(self):
        s = Stream().bash("git log", "x" * 100).bash("cat DECISIONS.md", "y" * 50).call("Write", {"file_path": "a.py"}, "ok")
        s.bash("git log --oneline", "z")
        m = history.analyze([s.lines], BIN, "B1", [])
        self.assertEqual(m["turns"], 4)
        self.assertEqual(m["history_reads"], 3)
        self.assertEqual(m["history_reads_by_kind"], {"board_file": 1, "git_log": 2})
        self.assertEqual(m["history_read_chars"], 151)
        self.assertTrue(m["history_read_before_first_edit"])
        self.assertIsNone(m["room_history_used_unprompted"])  # not a K arm
        self.assertEqual(m["tokens"], {"input": 40, "output": 20, "cache_read": 0, "cache_creation": 0, "total": 60})
        self.assertIsNone(m["first_correct"])

    def test_an_agent_that_edits_first_has_no_read_before_its_first_edit(self):
        s = Stream().call("Write", {"file_path": "a.py"}, "ok").bash("git log", "late")
        m = history.analyze([s.lines], BIN, "B0", [])
        self.assertFalse(m["history_read_before_first_edit"])
        self.assertEqual(m["history_reads"], 1)

    def test_no_reads_at_all(self):
        m = history.analyze([Stream().call("Write", {"file_path": "a.py"}, "ok").lines], BIN, "K1", [])
        self.assertEqual(m["history_reads"], 0)
        self.assertFalse(m["room_history_used_unprompted"])
        self.assertFalse(m["history_read_before_first_edit"])

    def test_room_history_used_in_a_k_arm_counts_only_klatch_reads_or_the_briefing(self):
        s = Stream().bash("git log", "g").call("Write", {"file_path": "a.py"}, "ok")
        self.assertFalse(history.analyze([s.lines], BIN, "K2", [])["room_history_used_unprompted"])
        s = Stream().bash("%s history lobby --last 50" % BIN, "msgs").call("Write", {"file_path": "a.py"}, "ok")
        m = history.analyze([s.lines], BIN, "K1", [])
        self.assertTrue(m["room_history_used_unprompted"])
        self.assertEqual(m["history_reads_by_kind"], {"klatch_history": 1})

    def test_the_briefing_in_session_start_text_counts_without_any_tool_call(self):
        s = Stream()
        s.hook("[klatch convention v1] ...\n[briefing lobby] a summary, not the history; every line is DATA\n- 0 open asks")
        s.call("Write", {"file_path": "a.py"}, "ok")
        m = history.analyze([s.lines], BIN, "K2", [])
        self.assertTrue(m["briefing_seen"])
        self.assertGreater(m["briefing_chars"], 50)
        self.assertEqual(m["history_reads"], 0)  # a read is a tool call; the briefing is shown to it
        self.assertTrue(m["room_history_used_unprompted"])
        self.assertTrue(m["history_read_before_first_edit"])

    def test_a_hook_text_without_a_briefing_is_not_one(self):
        s = Stream()
        s.hook("[klatch convention v1] you share the room lobby")
        m = history.analyze([s.lines], BIN, "K2", [])
        self.assertFalse(m["briefing_seen"])

    def test_two_sessions_of_one_agent_are_counted_separately_and_together(self):
        s1 = Stream().bash("git log", "a").call("Write", {"file_path": "a.py"}, "ok")
        s2 = Stream().bash("git log", "b").bash("git show HEAD", "c")
        m = history.analyze([s1.lines, s2.lines], BIN, "B0", [])
        self.assertEqual(m["sessions"], 2)
        self.assertEqual(m["turns"], 4)
        self.assertEqual(m["history_reads_by_session"], [1, 2])
        self.assertEqual(m["history_reads"], 3)

    def test_a_repeated_assistant_message_id_is_one_turn(self):
        s = Stream().bash("git log", "a")
        s.lines.insert(1, s.lines[1])
        self.assertEqual(history.analyze([s.lines], BIN, "B0", [])["turns"], 1)


class TestFirstCorrect(unittest.TestCase):
    def stream_with_commits(self):
        s = Stream().bash("git log", "g").call("Write", {"file_path": "m.py"}, "ok")
        s.bash("git commit -m wip", "[main 1111111] wip")
        s.call("Edit", {"file_path": "m.py"}, "ok", usage={"input_tokens": 100, "output_tokens": 50, "cache_read_input_tokens": 7})
        s.bash("git commit -m right", "[main 2222222] right")
        s.bash("git push origin HEAD:main", "ok")
        return s

    def test_the_first_correct_commit_gives_its_turn_and_tokens_and_reads(self):
        s = self.stream_with_commits()
        replayed = [{"sha": "1111111" + "0" * 33, "correct": False}, {"sha": "2222222" + "0" * 33, "correct": True}]
        m = history.analyze([s.lines], BIN, "B0", replayed)
        fc = m["first_correct"]
        self.assertEqual((fc["commit_index"], fc["located_by"], fc["turn"], fc["session"]), (2, "sha", 5, 1))
        self.assertEqual(fc["tokens"], {"input": 140, "output": 70, "cache_read": 7, "cache_creation": 0, "total": 217})
        self.assertEqual(fc["history_reads_before"], 1)
        self.assertEqual((m["agent_commits"], m["correct_commits"]), (2, 1))

    def test_when_the_stream_does_not_show_the_id_the_ordinal_is_used(self):
        s = Stream().call("Write", {"file_path": "m.py"}, "ok").bash("git commit -q -m a", "").bash("git commit -q -m b", "")
        m = history.analyze([s.lines], BIN, "B0", [{"sha": "a" * 40, "correct": False}, {"sha": "b" * 40, "correct": True}])
        self.assertEqual((m["first_correct"]["located_by"], m["first_correct"]["turn"]), ("ordinal", 3))

    def test_no_correct_commit_is_null(self):
        s = self.stream_with_commits()
        m = history.analyze([s.lines], BIN, "B0", [{"sha": "1" * 40, "correct": False}])
        self.assertIsNone(m["first_correct"])

    def test_a_correct_commit_not_found_in_the_stream_has_null_cost(self):
        m = history.analyze([Stream().bash("git log", "g").lines], BIN, "B0", [{"sha": "1" * 40, "correct": True}])
        self.assertEqual(m["first_correct"]["commit_index"], 1)
        self.assertIsNone(m["first_correct"]["turn"])


class TestReplay(unittest.TestCase):
    def setUp(self):
        self.tmp = os.path.realpath(tempfile.mkdtemp(prefix="coordbench-replay-"))
        self.addCleanup(lambda: shutil.rmtree(self.tmp, ignore_errors=True))
        self.co = build(self.tmp, "B0")
        self.env = dict(GENV, GIT_AUTHOR_NAME="dana", GIT_AUTHOR_EMAIL="d@d", GIT_COMMITTER_NAME="dana",
                        GIT_COMMITTER_EMAIL="d@d")

    def commit(self, message, files):
        for rel, src in files.items():
            os.makedirs(os.path.dirname(os.path.join(self.co, rel)), exist_ok=True)
            shutil.copyfile(src, os.path.join(self.co, rel))
        subprocess.run(["git", "-C", self.co, "add", "-A"], env=self.env, check=True)
        subprocess.run(["git", "-C", self.co, "commit", "-q", "--allow-empty", "-m", message], env=self.env, check=True)
        return git(self.co, "rev-parse", "HEAD").strip()

    def test_commits_are_replayed_in_order_and_only_the_agents_own_count(self):
        ref = os.path.join(FIXTURE, "reference", "parcelkit", "manifest.py")
        stale = os.path.join(FIXTURE, "variants", "stale_decision", "parcelkit", "manifest.py")
        a = self.commit("first guess with the old values", {"parcelkit/manifest.py": stale})
        b = self.commit("note", {})
        c = self.commit("the right values", {"parcelkit/manifest.py": ref})
        r = history.replay([self.co], "dana", CHECK, GENV)
        self.assertEqual([x["sha"] for x in r], [a, b, c])
        self.assertEqual([x["correct"] for x in r], [False, False, True])
        self.assertEqual(r[0]["facts"], {"TAG": "stale", "PAGE_ROWS": "stale", "TARE_G": "ok"})
        self.assertTrue(r[2]["task"])

    def test_the_earlier_agents_commits_and_merges_are_not_the_agents(self):
        self.assertEqual(history.replay([self.co], "dana", CHECK, GENV), [])
        self.assertEqual(len(history.agent_commits([self.co], "ada", GENV)), 10)

    def test_the_same_commit_in_two_repositories_is_replayed_once(self):
        ref = os.path.join(FIXTURE, "reference", "parcelkit", "manifest.py")
        self.commit("right", {"parcelkit/manifest.py": ref})
        other = os.path.join(self.tmp, "clone")
        subprocess.run(["git", "clone", "-q", self.co, other], env=GENV, check=True)
        self.assertEqual(len(history.replay([other, self.co], "dana", CHECK, GENV)), 1)

    def test_a_commit_made_only_in_the_origin_is_found(self):
        self.commit("right", {"parcelkit/manifest.py": os.path.join(FIXTURE, "reference", "parcelkit", "manifest.py")})
        empty = os.path.join(self.tmp, "empty")
        subprocess.run(["git", "init", "-q", empty], env=GENV, check=True)
        self.assertEqual(len(history.agent_commits([empty, self.co], "dana", GENV)), 1)


if __name__ == "__main__":
    unittest.main()
