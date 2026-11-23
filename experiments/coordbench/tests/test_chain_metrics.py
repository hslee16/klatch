"""Stream metrics of scenario S3 (#1368): idle and wasted turns of a blocked agent, and what the stream shows about claims
at the handoff. Streams here are built by hand in the shape `claude -p --output-format stream-json` prints; no model runs."""
import json
import os
import tempfile
import unittest

from helpers import BENCH  # noqa: F401
import chain
import scenario

DEP = "chainkit/schema.json"
GUARD = ["chainkit/loader.py"]
PULL_SHOWS_SCHEMA = " chainkit/schema.json | 12 ++++\n 1 file changed, 12 insertions(+)\n"


def bash(cmd, result=""):
    return ("Bash", {"command": cmd}, result)


def lines_of(turns):
    """JSON stream lines from turns: each turn is (text, [(tool, input, result), ...])."""
    out = [json.dumps({"type": "system", "subtype": "init", "model": "claude-fake"})]
    n = 0
    for k, (text, tools) in enumerate(turns, 1):
        blocks = []
        if text:
            blocks.append({"type": "text", "text": text})
        results = []
        for tool, inp, res in tools:
            n += 1
            blocks.append({"type": "tool_use", "id": "t%d" % n, "name": tool, "input": inp})
            results.append({"type": "tool_result", "tool_use_id": "t%d" % n, "content": res})
        out.append(json.dumps({"type": "assistant", "message": {"id": "m%d" % k, "content": blocks}}))
        if results:
            out.append(json.dumps({"type": "user", "message": {"content": results}}))
    return out


def report(turns, dep=DEP, guard=GUARD):
    return chain.blocked_report(chain.parse_stream(lines_of(turns)), dep, guard)


class TestBlockedReport(unittest.TestCase):
    def test_a_dependency_seen_on_the_first_turn_has_no_blocked_turns(self):
        r = report([("", [bash("git pull origin main", PULL_SHOWS_SCHEMA)]), ("done", [])])
        self.assertEqual((r["blocked_turns"], r["idle_turns"], r["wasted_turns"]), (0, 0, 0))
        self.assertTrue(r["dependency_seen"])
        self.assertEqual(r["dependency_seen_turn"], 1)
        self.assertEqual(r["turns"], 2)

    def test_polling_turns_before_the_dependency_are_idle(self):
        polls = [("", [bash("sleep 20")]), ("", [bash("klatch wait lobby", "nothing new")]),
                 ("", [bash("git fetch origin", "")]), ("", [bash("ls chainkit", "__init__.py")]),
                 ("", [("Read", {"file_path": "TASKS.md"}, "board")])]
        r = report(polls + [("", [bash("git pull origin main", PULL_SHOWS_SCHEMA)]), ("done", [])])
        self.assertEqual(r["idle_turns"], 5)
        self.assertEqual(r["blocked_turns"], 5)
        self.assertEqual(r["wasted_turns"], 5)
        self.assertEqual(r["dependency_seen_turn"], 6)

    def test_a_text_only_turn_while_blocked_is_idle(self):
        r = report([("I will wait for a.", []), ("", [bash("git pull", PULL_SHOWS_SCHEMA)])])
        self.assertEqual((r["idle_turns"], r["wasted_turns"]), (1, 1))

    def test_a_turn_that_does_other_work_is_blocked_but_not_idle(self):
        r = report([("", [("Read", {"file_path": "README.md"}, "the readme")]),
                    ("", [bash("python3 -m unittest discover -s tests -t .", "OK")]),
                    ("", [bash("sleep 5; python3 -m unittest", "OK")]),
                    ("", [bash("git pull", PULL_SHOWS_SCHEMA)])])
        self.assertEqual(r["blocked_turns"], 3)
        self.assertEqual((r["idle_turns"], r["guess_build_turns"], r["wasted_turns"]), (0, 0, 0))

    def test_a_turn_of_only_polls_in_one_command_is_idle(self):
        r = report([("", [bash("git fetch origin && git log origin/main --oneline | head -3", "abc msg")]),
                    ("", [bash("sleep 3; git status", "clean")]),
                    ("", [bash("git pull", PULL_SHOWS_SCHEMA)])])
        self.assertEqual(r["idle_turns"], 2)

    def test_writing_the_dependent_file_before_the_dependency_exists_is_a_guess_build(self):
        r = report([("", [("Write", {"file_path": "/w/chainkit/loader.py", "content": "x"}, "ok")]),
                    ("", [bash("cat > chainkit/loader.py <<'EOF'\nx\nEOF", "")]),
                    ("", [bash("sleep 5")]),
                    ("", [bash("git pull", PULL_SHOWS_SCHEMA)]),
                    ("", [("Write", {"file_path": "/w/chainkit/loader.py", "content": "y"}, "ok")])])
        self.assertEqual(r["guess_build_turns"], 2)
        self.assertEqual(r["idle_turns"], 1)
        self.assertEqual(r["wasted_turns"], 3)
        self.assertEqual(r["blocked_turns"], 3, "the write after the dependency is not blocked work")

    def test_a_missing_file_message_is_not_the_dependency(self):
        r = report([("", [bash("cat chainkit/schema.json", "cat: chainkit/schema.json: No such file or directory")]),
                    ("", [bash("ls chainkit", "__init__.py\nschema.json")])])
        self.assertEqual(r["dependency_seen_turn"], 2)
        self.assertEqual(r["blocked_turns"], 1)

    def test_a_message_that_names_the_file_is_not_the_file(self):
        r = report([("", [bash("klatch read lobby", "a: schema.json is on its way")]),
                    ("", [("SendMessage", {"message": "is schema.json ready?"}, "sent")]),
                    ("", [bash("git pull", PULL_SHOWS_SCHEMA)])])
        self.assertEqual(r["dependency_seen_turn"], 3)
        self.assertEqual(r["blocked_turns"], 2)
        self.assertEqual(r["idle_turns"], 1, "reading the room is a poll; sending a message is coordination, not idling")

    def test_a_dependency_that_never_shows_up_blocks_every_turn(self):
        r = report([("", [bash("sleep 5")]), ("", [bash("git fetch")]), ("gave up", [])])
        self.assertFalse(r["dependency_seen"])
        self.assertIsNone(r["dependency_seen_turn"])
        self.assertEqual((r["blocked_turns"], r["idle_turns"], r["turns"]), (3, 3, 3))

    def test_an_unreadable_or_empty_stream_gives_zeroes(self):
        r = chain.blocked_report(chain.parse_stream([]), DEP, GUARD)
        self.assertEqual((r["turns"], r["blocked_turns"], r["wasted_turns"]), (0, 0, 0))
        self.assertFalse(r["dependency_seen"])
        self.assertEqual(chain.parse_stream(["not json", "", "[1]"]), [])

    def test_streaming_duplicates_of_one_message_id_are_one_turn(self):
        first = json.dumps({"type": "assistant", "message": {"id": "m1", "content": [
            {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "sleep 5"}}]}})
        turns = chain.parse_stream([first, first])
        self.assertEqual(len(turns), 1)
        self.assertEqual(len(turns[0]["tools"]), 1)


def klatch_turns(agent_events):
    """Turns of one agent from [(verb-ish command, result)] strings."""
    return [("", [bash(c, r)]) for c, r in agent_events]


class TestHandoffClaims(unittest.TestCase):
    def streams(self, a, c):
        return {"a": chain.parse_stream(lines_of(klatch_turns(a))), "c": chain.parse_stream(lines_of(klatch_turns(c)))}

    def test_not_applicable_for_an_arm_without_claims(self):
        s = self.streams([("git push", "")], [("git pull", "")])
        self.assertIsNone(chain.handoff_claims(s, "B0", "a", "c", "klatch"))
        self.assertIsNone(chain.handoff_claims(s, "B1", "a", "c", "klatch"))

    def test_a_claim_released_before_the_handoff_message_is_a_release_not_a_transfer(self):
        a = [("klatch claim lobby chainkit/bins.py --reason 'normaliser'", "claimed"),
             ("klatch release lobby chainkit/bins.py", "released"),
             ("klatch post lobby 'checkpoint: parse_bin done'", "ok"),
             ("klatch post lobby 'handoff: c'", "ok")]
        c = [("klatch claim lobby chainkit/bins.py --reason 'finish'", "claimed")]
        h = chain.handoff_claims(self.streams(a, c), "K2", "a", "c", "klatch")
        self.assertEqual(h["a_claimed"], ["chainkit/bins.py"])
        self.assertEqual(h["a_released"], ["chainkit/bins.py"])
        self.assertEqual(h["a_released_before_handoff"], ["chainkit/bins.py"])
        self.assertEqual(h["a_still_holding"], [])
        self.assertEqual(h["c_claimed"], ["chainkit/bins.py"])
        self.assertTrue(h["handoff_message"])
        self.assertTrue(h["checkpoint_message"])
        self.assertIsNone(h["transferred_by_klatch"])
        self.assertIn("store", h["note"])

    def test_a_claim_still_held_when_a_stops_is_dangling(self):
        a = [("klatch claim lobby chainkit/bins.py", "claimed"), ("klatch post lobby 'handoff: c'", "ok")]
        c = [("klatch claim lobby chainkit/bins.py", "refused: held by a")]
        h = chain.handoff_claims(self.streams(a, c), "K1", "a", "c", "klatch")
        self.assertEqual(h["a_still_holding"], ["chainkit/bins.py"])
        self.assertEqual(h["a_released_before_handoff"], [])
        self.assertEqual(h["c_claim_refused"], ["chainkit/bins.py"])
        self.assertTrue(h["handoff_message"])
        self.assertFalse(h["checkpoint_message"])

    def test_a_release_after_the_handoff_message_is_counted_apart(self):
        a = [("klatch claim lobby f.py", "claimed"), ("klatch post lobby 'handoff: c'", "ok"),
             ("klatch release lobby f.py", "released")]
        h = chain.handoff_claims(self.streams(a, []), "K2", "a", "c", "klatch")
        self.assertEqual(h["a_released"], ["f.py"])
        self.assertEqual(h["a_released_before_handoff"], [])


class TestCompute(unittest.TestCase):
    def test_compute_reads_the_stream_files_of_a_run_by_the_scenario_file(self):
        scn = scenario.load("s3")
        with tempfile.TemporaryDirectory() as d:
            def put(name, turns):
                with open(os.path.join(d, name + ".stream.jsonl"), "w") as f:
                    f.write("\n".join(lines_of(turns)) + "\n")
            put("a", [("", [bash("git push origin HEAD:main", "")])])
            put("b", [("", [bash("sleep 5")]), ("", [bash("git pull", PULL_SHOWS_SCHEMA)])])
            put("c", [("", [bash("git fetch")]), ("", [bash("git pull", " chainkit/loader.py | 9 +\n")])])
            out = chain.compute(d, scn, "B0", None)
        self.assertEqual(sorted(out["blocked"]), ["b", "c"])
        self.assertEqual(out["blocked"]["b"]["idle_turns"], 1)
        self.assertEqual(out["blocked"]["c"]["idle_turns"], 1)
        self.assertIsNone(out["handoff_claims"])

    def test_a_missing_stream_file_gives_a_null_entry_not_a_crash(self):
        scn = scenario.load("s3")
        with tempfile.TemporaryDirectory() as d:
            out = chain.compute(d, scn, "B0", None)
        self.assertEqual(out["blocked"], {"b": None, "c": None})


if __name__ == "__main__":
    unittest.main()
