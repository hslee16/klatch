"""recovery.py (#1371) on hand-built streams: the kill conditions the harness evaluates, and the recovery metrics, including
how the survivors noticed (the marker list is klatch's own output text). No process is started."""
import json
import unittest

from helpers import BENCH  # noqa: F401
import recovery
import scenario

SCN = scenario.load("s6")
KB = "/x/klatch"


class Stream:
    """Builds the stream lines of one agent."""

    def __init__(self):
        self.lines, self.n = [], 0

    def _emit(self, o):
        self.lines.append(json.dumps(o))

    def use(self, name, inp, result=""):
        self.n += 1
        tid = "t%d" % self.n
        self._emit({"type": "assistant", "message": {"id": "m%d" % self.n, "content": [{"type": "tool_use", "id": tid, "name": name, "input": inp}]}})
        self._emit({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": tid, "content": result}]}})
        return self

    def bash(self, cmd, result=""):
        return self.use("Bash", {"command": cmd}, result)

    def klatch(self, args, result=""):
        return self.bash("%s %s" % (KB, args), result)

    def write(self, path):
        return self.use("Write", {"file_path": path, "content": "x"}, "ok")

    def hook(self, text):
        self._emit({"type": "system", "subtype": "hook_response", "hook_event": "PostToolUse", "output": text})
        return self


ASKTEXT = "[klatch] a: seeking: what is the bit numbering of a mask?"


class TestVictimState(unittest.TestCase):
    def state(self, s, ask_re=SCN["kill_agent"]["ask_pattern"]):
        return recovery.victim_state(s.lines, ["recoverkit/bits.py"], ask_re)

    def test_edit_claim_and_turns(self):
        s = Stream().klatch("claim lobby recoverkit/bits.py --reason 'my module'").write("recoverkit/bits.py")
        st = self.state(s)
        self.assertTrue(st["edited"])
        self.assertEqual(st["claims"], {"recoverkit/bits.py"})
        self.assertEqual(st["turns"], 2)

    def test_an_absolute_path_and_a_shell_write_both_count_as_an_edit(self):
        self.assertTrue(self.state(Stream().write("/tmp/w/clones/c/recoverkit/bits.py"))["edited"])
        self.assertTrue(self.state(Stream().bash("cat > recoverkit/bits.py <<EOF\nx\nEOF"))["edited"])
        self.assertFalse(self.state(Stream().bash("cat recoverkit/bits.py"))["edited"], "a read is not an edit")
        self.assertFalse(self.state(Stream().write("recoverkit/wrap.py"))["edited"], "another module is not its own")

    def test_a_release_drops_the_claim_and_an_ask_claim_is_kept_apart(self):
        s = Stream().klatch("claim lobby recoverkit/bits.py").klatch("claim lobby ask:4").klatch("release lobby recoverkit/bits.py")
        st = self.state(s)
        self.assertEqual(st["claims"], set())
        self.assertEqual(st["ask_claims"], {"ask:4"})

    def test_a_claim_with_a_reason_flag_takes_the_path_not_the_reason(self):
        s = Stream().klatch("claim lobby --reason 'why not' recoverkit/bits.py")
        self.assertEqual(self.state(s)["claims"], {"recoverkit/bits.py"})

    def test_the_question_arrives_in_a_hook_and_stays_open_until_answered(self):
        s = Stream().hook(ASKTEXT)
        self.assertTrue(self.state(s)["ask_open"])
        s.klatch('post lobby "answer to #3: index 0 is the msb"')
        st = self.state(s)
        self.assertFalse(st["ask_open"])
        self.assertTrue(st["ask_seen"])

    def test_klatch_answer_closes_the_ask_and_drops_its_claim(self):
        s = Stream().klatch("claim lobby ask:3").hook(ASKTEXT).klatch("answer lobby 3 'index 0 is the msb'")
        st = self.state(s)
        self.assertFalse(st["ask_open"])
        self.assertEqual(st["ask_claims"], set())

    def test_the_echo_of_the_victims_own_message_does_not_reopen_the_ask(self):
        s = Stream().hook(ASKTEXT).klatch('post lobby "answer to #3: bit numbering is msb first"', "posted: seeking? numbering")
        self.assertFalse(self.state(s)["ask_open"])

    def test_a_question_that_is_not_the_ask_does_not_open_it(self):
        s = Stream().hook("[klatch] b: seeking: which test runner do we use?")
        self.assertFalse(self.state(s)["ask_open"])
        s = Stream().bash("cat docs/FORMATS.md", "Bit indexes count from the most significant bit")
        self.assertFalse(self.state(s)["ask_open"], "reading the convention is not being asked about it")


class StubProbe:
    def __init__(self, board=True, committed=False):
        self.board, self.comm = board, committed
        self.calls = 0

    def board_working(self, victim):
        self.calls += 1
        return self.board

    def committed(self, victim, paths):
        self.calls += 1
        return self.comm


class TestKiller(unittest.TestCase):
    def killer(self, arm, probe=None, spec=None):
        return recovery.Killer(spec or SCN["kill_agent"], arm, ["recoverkit/bits.py"], probe or StubProbe(), git_every_s=0)

    def test_k_arm_conditions_all_hold(self):
        s = Stream().klatch("claim lobby recoverkit/bits.py").hook(ASKTEXT).write("recoverkit/bits.py")
        due = self.killer("K2").poll(s.lines)
        self.assertEqual(due["trigger"], "conditions")
        self.assertEqual(sorted(due["conditions"]), ["ask_open", "claimed", "edited", "no_commit"])

    def test_nothing_fires_while_one_condition_is_missing(self):
        base = Stream().klatch("claim lobby recoverkit/bits.py").hook(ASKTEXT)
        self.assertIsNone(self.killer("K2").poll(base.lines), "not edited yet")
        s = Stream().hook(ASKTEXT).write("recoverkit/bits.py")
        self.assertIsNone(self.killer("K2").poll(s.lines), "no claim yet")
        s = Stream().klatch("claim lobby recoverkit/bits.py").write("recoverkit/bits.py")
        self.assertIsNone(self.killer("K2").poll(s.lines), "the ask has not arrived")

    def test_b1_needs_the_board_line_on_origin_and_no_commit_of_the_module(self):
        s = Stream().hook(ASKTEXT).write("recoverkit/bits.py")
        self.assertIsNone(self.killer("B1", StubProbe(board=False)).poll(s.lines))
        self.assertIsNone(self.killer("B1", StubProbe(committed=True)).poll(s.lines), "it already committed the module")
        self.assertEqual(self.killer("B1", StubProbe()).poll(s.lines)["trigger"], "conditions")

    def test_b0_has_only_the_edit_and_the_git_condition(self):
        s = Stream().write("recoverkit/bits.py")
        due = self.killer("B0").poll(s.lines)
        self.assertEqual(sorted(due["conditions"]), ["edited", "no_commit"])

    def test_git_is_not_asked_until_the_stream_conditions_hold(self):
        probe = StubProbe()
        self.assertIsNone(self.killer("B1", probe).poll(Stream().bash("ls").lines))
        self.assertEqual(probe.calls, 0)

    def test_the_fallback_fires_at_the_turn_count_and_says_so(self):
        s = Stream()
        for _ in range(13):
            s.bash("ls")
        self.assertIsNone(self.killer("B0").poll(s.lines))
        s.bash("ls")
        due = self.killer("B0").poll(s.lines)
        self.assertEqual((due["trigger"], due["turns"]), ("fallback", 14))

    def test_the_turns_condition(self):
        spec = {"agent": "c", "after": [{"kind": "turns", "min": 3}]}
        k = self.killer("B0", spec=spec)
        s = Stream().bash("ls").bash("ls")
        self.assertIsNone(k.poll(s.lines))
        self.assertEqual(k.poll(s.bash("ls").lines)["trigger"], "conditions")

    def test_an_unknown_condition_is_an_error_not_a_silent_pass(self):
        k = self.killer("B0", spec={"agent": "c", "after": [{"kind": "moon"}]})
        with self.assertRaises(ValueError):
            k.poll(Stream().bash("ls").lines)

    def test_the_ask_taken_condition_is_a_klatch_claim_of_an_ask(self):
        spec = {"agent": "c", "after": [{"kind": "ask_taken", "arms": ["K2"]}]}
        k = self.killer("K2", spec=spec)
        self.assertIsNone(k.poll(Stream().klatch("claim lobby recoverkit/bits.py").lines))
        self.assertEqual(k.poll(Stream().klatch("claim lobby ask:3").lines)["trigger"], "conditions")


def timeline(s, start=0.0, step=1.0):
    return [round(start + step * i, 3) for i in range(len(s.lines))]


def analyze(streams, kill_lines, arm="K2", kill_t=10.0, times=None, victim_lines=None):
    """streams: {agent: Stream}; kill_lines: {agent: lines read at the kill}."""
    sessions = {k: v.lines for k, v in streams.items()}
    tl = times if times is not None else {k: timeline(v, 10.0 - kill_lines.get(k, 0) * 1.0) for k, v in streams.items()}
    kill = {"fired": True, "agent": "c", "t": kill_t, "turns": 3, "trigger": "conditions", "lines": kill_lines}
    return recovery.analyze(sessions, tl, SCN, arm, kill)


class TestAnalyze(unittest.TestCase):
    def pre(self, s):
        """Two harmless turns before the kill."""
        return s.bash("git status").bash("ls")

    def test_the_kill_that_did_not_fire_has_no_metrics(self):
        self.assertEqual(recovery.analyze({"a": []}, {}, SCN, "B0", {"fired": False}), {"kill_fired": False})
        self.assertEqual(recovery.analyze({"a": []}, {}, SCN, "B0", None), {"kill_fired": False})

    def test_first_action_is_the_first_write_claim_answer_or_message_about_the_victims_work_after_the_kill(self):
        a = self.pre(Stream())
        a_cut = len(a.lines)
        a.bash("git log").klatch("claim lobby recoverkit/bits.py", "claimed recoverkit/bits.py for 600s (advisory)")
        b = self.pre(Stream())
        b_cut = len(b.lines)
        b.bash("ls").bash("ls").write("recoverkit/bits.py")
        m = analyze({"a": a, "b": b, "c": Stream()}, {"a": a_cut, "b": b_cut})
        fa = m["first_survivor_action"]
        self.assertEqual((fa["agent"], fa["kind"]), ("a", "claim"))
        self.assertEqual(fa["turn_after_kill"], 2, "git log, then the claim")
        self.assertEqual(m["time_to_first_survivor_action_s"], m["first_survivor_action"]["t_after_kill_s"])
        self.assertGreater(m["time_to_first_survivor_action_s"], 0)
        self.assertGreaterEqual(m["survivor_turns_to_first_action"], 2)

    def test_reading_the_victims_file_or_polling_is_not_an_action(self):
        a = self.pre(Stream())
        cut = len(a.lines)
        a.bash("cat recoverkit/bits.py").klatch("claims lobby").klatch("wait lobby").bash("git log -- recoverkit/bits.py")
        m = analyze({"a": a, "b": Stream(), "c": Stream()}, {"a": cut})
        self.assertIsNone(m["first_survivor_action"])
        self.assertIsNone(m["time_to_first_survivor_action_s"])
        self.assertIsNone(m["survivor_turns_to_first_action"])

    def test_events_before_the_kill_do_not_count_as_after(self):
        a = Stream().write("recoverkit/bits.py")  # before the kill: a false alarm, not a recovery
        cut = len(a.lines)
        m = analyze({"a": a, "b": Stream(), "c": Stream()}, {"a": cut})
        self.assertIsNone(m["first_survivor_action"])
        self.assertEqual(m["false_alarms"], 1)

    def test_a_message_naming_the_victim_or_its_module_is_an_action_and_an_answer_is_one(self):
        b = Stream()
        m = analyze({"a": Stream(), "b": b.klatch('post lobby "c seems gone, taking bits"'), "c": Stream()}, {"b": 0})
        self.assertEqual(m["first_survivor_action"]["kind"], "message")
        b2 = Stream().klatch('post lobby "answer to #3: index 0 is the msb"')
        m = analyze({"a": Stream(), "b": b2, "c": Stream()}, {"b": 0})
        self.assertEqual(m["first_survivor_action"]["kind"], "answer")
        self.assertIsNotNone(m["time_to_ask_answer_s"])
        b3 = Stream().klatch('post lobby "working: wrap"')
        self.assertIsNone(analyze({"a": Stream(), "b": b3, "c": Stream()}, {"b": 0})["first_survivor_action"])

    def test_noticing_markers_are_klatchs_own_text(self):
        for text, marker in [
            ("dead_holder room lobby claim recoverkit/bits.py holder c", "stuck_dead_holder"),
            ("mediator proposal: @c holds recoverkit/bits.py in lobby but its process has exited. The claim lapses by its own time", "stuck_dead_holder"),
            ("stale_ask ask #3 asker a age 140s", "stuck_stale_ask"),
            ("  your ask #3: c's claim lapsed without an answer; the ask is open again: seeking: what is the bit numbering", "ask_lease_lapsed"),
        ]:
            a = Stream().klatch("stuck", text) if marker != "ask_lease_lapsed" else Stream().hook(text)
            m = analyze({"a": a, "b": Stream(), "c": Stream()}, {"a": 0})
            self.assertEqual(m["noticed_by"], [marker], text)
            self.assertEqual(m["first_notice"]["marker"], marker)
            self.assertEqual(m["first_notice"]["agent"], "a")

    def test_a_marker_about_another_member_is_not_a_notice_of_the_victims_death(self):
        a = Stream().klatch("stuck", "dead_holder room lobby claim src/x.py holder b").hook("your ask #3: b's claim lapsed without an answer; the ask is open again")
        m = analyze({"a": a, "b": Stream(), "c": Stream()}, {"a": 0})
        self.assertEqual(m["noticed_by"], [])

    def test_a_claim_that_succeeds_on_the_victims_path_after_the_kill_means_its_lease_ran_out(self):
        a = Stream().klatch("claim lobby recoverkit/bits.py", "claimed recoverkit/bits.py for 600s (advisory)")
        m = analyze({"a": a, "b": Stream(), "c": Stream()}, {"a": 0})
        self.assertEqual(m["noticed_by"], ["claim_taken_after_lapse"])
        refused = Stream().klatch("claim lobby recoverkit/bits.py", "recoverkit/bits.py is claimed by c in room lobby (412s left)")
        m = analyze({"a": refused, "b": Stream(), "c": Stream()}, {"a": 0})
        self.assertEqual(m["noticed_by"], [], "a refused claim is the holder still alive as far as klatch knows")
        self.assertEqual(m["sightings"], ["claim_held_by_victim"])

    def test_how_noticed_is_the_marker_that_came_first_or_no_marker(self):
        a = Stream().klatch("stuck", "dead_holder room lobby claim recoverkit/bits.py holder c").write("recoverkit/bits.py")
        m = analyze({"a": a, "b": Stream(), "c": Stream()}, {"a": 0})
        self.assertEqual(m["how_noticed"], "stuck_dead_holder")
        self.assertEqual(m["first_survivor_action"]["kind"], "edit")
        a = Stream().write("recoverkit/bits.py").klatch("stuck", "dead_holder room lobby claim recoverkit/bits.py holder c")
        m = analyze({"a": a, "b": Stream(), "c": Stream()}, {"a": 0})
        self.assertEqual(m["how_noticed"], "no-marker", "it acted before any marker was shown")
        self.assertEqual(analyze({"a": Stream(), "b": Stream(), "c": Stream()}, {})["how_noticed"], None)

    def test_the_markers_are_for_the_k_arms_only(self):
        a = Stream().klatch("stuck", "dead_holder room lobby claim recoverkit/bits.py holder c").write("recoverkit/bits.py")
        for arm in ("B0", "B1"):
            m = analyze({"a": a, "b": Stream(), "c": Stream()}, {"a": 0}, arm=arm)
            self.assertIsNone(m["noticed_by"])
            self.assertIsNone(m["how_noticed"])
            self.assertIsNone(m["notice_markers"])
            self.assertIsNotNone(m["first_survivor_action"])

    def test_the_stale_board_line_is_a_sighting_not_a_notice(self):
        a = Stream().bash("cat TASKS.md", "# Task board\n- c | working | recoverkit/bits.py | bits\n")
        m = analyze({"a": a, "b": Stream(), "c": Stream()}, {"a": 0}, arm="B1")
        self.assertEqual(m["sightings"], ["stale_board_entry"])

    def test_false_alarm_is_a_takeover_of_a_live_agents_module(self):
        b = Stream().write("recoverkit/ranges.py").klatch("claim lobby recoverkit/ranges.py")
        m = analyze({"a": Stream(), "b": b, "c": Stream()}, {"b": 0})
        self.assertEqual(m["false_alarms"], 1, "one pair, however many actions")
        self.assertEqual(m["false_alarm_detail"][0]["owner"], "a")
        self.assertFalse(m["false_alarm_detail"][0]["victim_unit"])

    def test_the_owner_touching_its_own_module_and_a_survivor_recovering_after_the_kill_are_no_false_alarm(self):
        a = Stream().write("recoverkit/ranges.py")
        b = Stream().write("recoverkit/wrap.py").write("recoverkit/bits.py")
        m = analyze({"a": a, "b": b, "c": Stream()}, {"a": 0, "b": 1})
        self.assertEqual(m["false_alarms"], 0, m["false_alarm_detail"])

    def test_no_survivor_turns_when_nothing_happens_after_the_kill(self):
        a = self.pre(Stream())
        m = analyze({"a": a, "b": Stream(), "c": Stream()}, {"a": len(a.lines)})
        self.assertEqual(m["survivor_turns_after_kill"], 0)

    def test_without_a_timeline_the_times_are_none_and_nothing_crashes(self):
        a = Stream().write("recoverkit/bits.py")
        m = analyze({"a": a, "b": Stream(), "c": Stream()}, {"a": 0}, times={})
        self.assertEqual(m["first_survivor_action"]["kind"], "edit")
        self.assertIsNone(m["time_to_first_survivor_action_s"])


if __name__ == "__main__":
    unittest.main()
