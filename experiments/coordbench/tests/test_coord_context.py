"""#1521 and #1524: the coordination context a stream carried and the idle turns, counted the same way in every arm.
Streams are written by hand in the shape of `claude -p --output-format stream-json --verbose --include-hook-events`; a
K arm and a B1 arm that do the equivalent thing must give equal numbers. No model, no spend."""
import json
import unittest

from helpers import BENCH  # noqa: F401
import cmdkinds
import ctxcount
import streams
from test_streams import asst, ev, res, tool

KB = "/opt/klatch/klatch"
_ANSWER = ("answer: the retry limit is 7 and it lives in config/limits.toml under the key [net] retries. "
          "I checked both the default and the override file; the override wins when both are set, and a value above 9 "
          "is clamped at load time, so seven is safe. The test for it is tests/test_limits.py::test_clamp, which passes "
          "on main as of my last pull. Ask again if you need the commit that introduced the clamp, I have it noted, and the review thread too.")
ANSWER = (_ANSWER + " Noted." * 80)[:458]  # exactly 458 characters, as the review's example
INIT_B1 = ev(type="system", subtype="init", model="claude-haiku-4-5-20251001", tools=["Bash", "Edit", "SendMessage", "ListAgents"])
INIT_K = ev(type="system", subtype="init", model="claude-haiku-4-5-20251001", tools=["Bash", "Edit"])


def inbound(text):
    return ev(type="user", message={"role": "user", "content": [{"type": "text", "text": text}]})


def hook(text):
    return ev(type="system", subtype="hook_response", hook_event="PostToolUse", output=text)


def bash(mid, tid, cmd, out, usage=None):
    return [asst(mid, [tool(tid, "Bash", command=cmd)], usage), res(tid, out)]


class SameAnswerBothWays(unittest.TestCase):
    def test_the_fixture_answer_is_458_characters(self):
        self.assertEqual(len(ANSWER), 458)

    def test_inbound_message_and_hook_digest_count_the_same(self):
        b1 = streams.parse_stream([asst("m0", [{"type": "text", "text": "start"}]), inbound(ANSWER)], klatch_bin=KB)
        k2 = streams.parse_stream([asst("m0", [{"type": "text", "text": "start"}]), hook(ANSWER)], klatch_bin=KB)
        self.assertEqual(b1["coord_context_chars"], 458)
        self.assertEqual(k2["coord_context_chars"], 458)
        self.assertEqual(ctxcount.tokens_of_chars(b1["coord_context_chars"]), ctxcount.tokens_of_chars(k2["coord_context_chars"]))
        self.assertEqual(b1["coord_context_parts"]["inbound"], 458)
        self.assertEqual(k2["coord_context_parts"]["hook"], 458)

    def test_an_inbound_message_is_not_counted_as_the_task_prompt(self):
        s = streams.parse_stream([inbound("the task prompt before any assistant turn")], klatch_bin=KB)
        self.assertEqual(s["coord_context_chars"], 0)

    def test_a_pulled_answer_costs_the_same_as_a_pushed_one_plus_the_call(self):
        # K1 reads the answer with klatch read; B1 reads it from TASKS.md: result characters plus the call's own input
        k = streams.parse_stream(bash("m1", "t1", KB + " read lobby", ANSWER), klatch_bin=KB)
        b = streams.parse_stream(bash("m1", "t1", "cat TASKS.md", ANSWER), klatch_bin=KB)
        self.assertEqual(k["coord_context_parts"]["tool_results"], 458)
        self.assertEqual(b["coord_context_parts"]["tool_results"], 458)
        self.assertEqual(k["coord_context_parts"]["tool_calls"] > 0, True)
        self.assertEqual(b["coord_context_parts"]["tool_calls"] > 0, True)

    def test_every_summary_says_how_the_tokens_were_estimated(self):
        s = streams.parse_stream([INIT_K], klatch_bin=KB)
        self.assertEqual(s["coord_context_method"], {"method": "chars/4", "chars_per_token": 4, "exact": False})


class ToolSchemas(unittest.TestCase):
    def test_b1_carries_the_schemas_of_its_two_messaging_tools(self):
        s = streams.parse_stream([INIT_B1], klatch_bin=KB)
        want = ctxcount.SCHEMA_CHARS["SendMessage"] + ctxcount.SCHEMA_CHARS["ListAgents"]
        self.assertEqual(s["coord_context_parts"]["schema"], want)
        self.assertEqual(s["coord_context_chars"], want)
        self.assertEqual(s["schema_source"], "pinned-estimate")

    def test_a_cli_arm_carries_no_coordination_schema(self):
        s = streams.parse_stream([INIT_K], klatch_bin=KB)
        self.assertEqual(s["coord_context_parts"]["schema"], 0)

    def test_a_klatch_mcp_tool_in_the_init_list_is_counted_like_any_coordination_tool(self):
        init = ev(type="system", subtype="init", tools=["Bash", "mcp__klatch__post", "mcp__other__thing"])
        s = streams.parse_stream([init], klatch_bin=KB)
        self.assertEqual(s["coord_context_parts"]["schema"], ctxcount.SCHEMA_CHARS["mcp__klatch"])

    def test_a_schema_the_stream_carries_is_measured_not_estimated(self):
        sch = {"name": "SendMessage", "description": "x" * 100, "input_schema": {"type": "object"}}
        init = ev(type="system", subtype="init", tools=[sch, "Bash"])
        s = streams.parse_stream([init], klatch_bin=KB)
        self.assertEqual(s["coord_context_parts"]["schema"], len(json.dumps(sch)))
        self.assertEqual(s["schema_source"], "stream")

    def test_a_repeated_init_event_is_counted_once(self):
        s = streams.parse_stream([INIT_B1, INIT_B1], klatch_bin=KB)
        self.assertEqual(s["coord_context_parts"]["schema"],
                         ctxcount.SCHEMA_CHARS["SendMessage"] + ctxcount.SCHEMA_CHARS["ListAgents"])


class UsageExact(unittest.TestCase):
    def u(self, inp, out, cache_read=0, cache_creation=0):
        return {"input_tokens": inp, "output_tokens": out, "cache_read_input_tokens": cache_read,
                "cache_creation_input_tokens": cache_creation}

    def test_the_context_growth_after_a_coordination_only_gap_is_the_exact_cost(self):
        lines = [asst("m1", [tool("t1", "Bash", command=KB + " read lobby")], self.u(1000, 20)), res("t1", ANSWER),
                 asst("m2", [{"type": "text", "text": "ok"}], self.u(1000, 5, cache_read=0, cache_creation=0))]
        # m2's context is 1000 + 20 (m1's output) + 130 (the answer): make it so
        lines[2] = asst("m2", [{"type": "text", "text": "ok"}], self.u(1150, 5))
        s = streams.parse_stream(lines, klatch_bin=KB)
        ex = s["usage_exact"]
        self.assertEqual(ex["method"], "usage-delta")
        self.assertEqual(ex["tokens"], 130)
        self.assertEqual(ex["gaps"], 1)

    def test_a_gap_that_also_carried_other_tool_output_is_not_attributed(self):
        lines = [asst("m1", [tool("t1", "Bash", command=KB + " read lobby"), tool("t2", "Edit", file_path="a.py")], self.u(1000, 20)),
                 res("t1", ANSWER), res("t2", "File edited"), asst("m2", [{"type": "text", "text": "ok"}], self.u(1160, 5))]
        s = streams.parse_stream(lines, klatch_bin=KB)
        self.assertEqual(s["usage_exact"]["tokens"], 0)
        self.assertEqual(s["usage_exact"]["gaps"], 0)

    def test_no_usage_means_no_exact_figure_never_zero(self):
        lines = [json.dumps({"type": "assistant", "message": {"id": "m1", "content": [tool("t1", "Bash", command=KB + " read x")]}}),
                 res("t1", "hello"),
                 json.dumps({"type": "assistant", "message": {"id": "m2", "content": [{"type": "text", "text": "x"}]}})]
        s = streams.parse_stream(lines, klatch_bin=KB)
        self.assertIsNone(s["usage_exact"]["tokens"])

    def test_first_turn_context_is_recorded_for_schema_differences_between_arms(self):
        s = streams.parse_stream([asst("m1", [{"type": "text", "text": "x"}], self.u(10, 2, 300, 40))], klatch_bin=KB)
        self.assertEqual(s["usage_exact"]["first_turn_context_tokens"], 350)


class ResolvedCommands(unittest.TestCase):
    def count(self, cmd, **kw):
        return streams.parse_stream([asst("m1", [tool("t1", "Bash", command=cmd)])], klatch_bin=KB, **kw)

    def test_a_shell_variable_holding_the_binary_path_is_resolved(self):
        s = self.count('K=%s; "$K" post lobby "working: alpha"' % KB)
        self.assertEqual(s["messages"], 1)
        s = self.count('export K="%s"\n${K} post lobby hi' % KB)
        self.assertEqual(s["messages"], 1)

    def test_a_function_or_alias_wrapper_defined_in_the_command_is_resolved(self):
        self.assertEqual(self.count('k() { %s "$@"; }; k dm bob hi' % KB)["messages"], 1)
        self.assertEqual(self.count("alias kk='%s'; kk ask lobby why" % KB)["messages"], 1)

    def test_an_env_prefix_is_matched(self):
        self.assertEqual(self.count("KLATCH_NAME=a %s post lobby hi" % KB)["messages"], 1)

    def test_a_wrapper_the_command_does_not_define_is_the_stated_limit(self):
        self.assertEqual(self.count("./k.sh post lobby hi")["messages"], 0)
        self.assertEqual(self.count('"$K" post lobby hi')["messages"], 0)
        self.assertIn("limit", cmdkinds.RESOLVE_LIMIT.lower())

    def test_resolution_is_exact_text_substitution(self):
        r = cmdkinds.resolve_command('K=%s; "$K" read lobby' % KB, KB)
        self.assertIn(KB + " read lobby", r)


class BoardEditsAreMessages(unittest.TestCase):
    def count(self, name, **inp):
        return streams.parse_stream([asst("m1", [tool("t1", name, **inp)])], klatch_bin=KB)

    def test_edits_and_writes_that_add_a_message_line_to_the_board_count(self):
        ans = "answer: X=1 (from b)"
        self.assertEqual(self.count("Edit", file_path="/w/TASKS.md", old_string="working", new_string=ans)["messages"], 1)
        self.assertEqual(self.count("Write", file_path="TASKS.md", content=ans)["messages"], 1)
        self.assertEqual(self.count("Bash", command="echo '%s' >> TASKS.md" % ans)["messages"], 1)
        self.assertEqual(self.count("Bash", command="printf 'seeking: q' | tee -a TASKS.md")["messages"], 1)
        self.assertEqual(self.count("Bash", command="echo x >> TASKS.md")["board_edits"], 1)

    def test_a_claim_or_status_edit_is_a_board_edit_and_not_a_message(self):
        self.assertEqual(self.count("Edit", file_path="/w/TASKS.md", old_string="working", new_string="done")["messages"], 0)
        self.assertEqual(self.count("Bash", command="echo '- a | working | f.py | t' >> TASKS.md")["messages"], 0)
        self.assertEqual(self.count("Bash", command="sed -i 's/working/done/' TASKS.md")["board_edits"], 1)

    def test_reading_the_board_is_not_a_message(self):
        self.assertEqual(self.count("Read", file_path="/w/TASKS.md")["messages"], 0)
        self.assertEqual(self.count("Bash", command="cat TASKS.md")["messages"], 0)
        self.assertEqual(self.count("Bash", command="grep working TASKS.md")["messages"], 0)

    def test_an_edit_of_another_file_is_not_a_board_edit(self):
        self.assertEqual(self.count("Edit", file_path="a.py", old_string="x", new_string="y")["board_edits"], 0)

    def test_a_message_and_a_board_edit_both_count_once_each(self):
        s = streams.parse_stream([asst("m1", [tool("t1", "SendMessage", to="b", message="hi"),
                                              tool("t2", "Edit", file_path="TASKS.md", old_string="a", new_string="answer: other fact (from c)")])])
        self.assertEqual(s["messages"], 2)


EMPTY_READ = "klatch: no unread messages here. Messages a hook or an earlier read already showed are not shown again"

K_TURNS = (
    [asst("m1", [tool("t1", "Bash", command=KB + " read lobby")]), res("t1", "a: starting units")]
    + [asst("m2", [tool("t2", "Bash", command=KB + " read lobby")]), res("t2", EMPTY_READ)]
    + [asst("m3", [tool("t3", "Bash", command=KB + " wait lobby --timeout 60")]), res("t3", "timed out after 60 s")]
    + [asst("m4", [tool("t4", "Bash", command=KB + " who")]), res("t4", "a\nb")]
    + [asst("m5", [tool("t5", "Bash", command=KB + " who")]), res("t5", "a\nb")]
    + [asst("m6", [tool("t6", "Bash", command=KB + " post lobby 'working: x'")]), res("t6", "posted")]
)
B1_TURNS = (
    [asst("m1", [tool("t1", "Bash", command="git pull --no-rebase origin main")]), res("t1", "Updating 1a2b..3c4d\nFast-forward\n TASKS.md | 1 +")]
    + [asst("m2", [tool("t2", "Bash", command="git pull --no-rebase origin main")]), res("t2", "Already up to date.")]
    + [asst("m3", [tool("t3", "Bash", command="sleep 20")]), res("t3", "")]
    + [asst("m4", [tool("t4", "ListAgents")]), res("t4", "a\nb")]
    + [asst("m5", [tool("t5", "ListAgents")]), res("t5", "a\nb")]
    + [asst("m6", [tool("t6", "SendMessage", to="b", message="working: x")]), res("t6", "sent")]
)


class IdleTurns(unittest.TestCase):
    def test_equivalent_k_and_b1_sequences_give_equal_idle_turns_and_messages(self):
        k = streams.parse_stream(K_TURNS, klatch_bin=KB)
        b = streams.parse_stream(B1_TURNS, klatch_bin=KB)
        self.assertEqual((k["idle_turns"], k["messages"]), (3, 1))
        self.assertEqual((b["idle_turns"], b["messages"]), (3, 1))
        self.assertEqual(k["turns"], b["turns"])

    def test_a_read_that_returned_something_new_is_not_idle(self):
        s = streams.parse_stream(K_TURNS[:2], klatch_bin=KB)
        self.assertEqual(s["idle_turns"], 0)  # the first klatch read returned a message
        s = streams.parse_stream(B1_TURNS[:2], klatch_bin=KB)
        self.assertEqual(s["idle_turns"], 0)  # the first pull brought a change

    def test_the_same_board_read_twice_is_idle_the_second_time(self):
        lines = (bash("m1", "t1", "cat TASKS.md", "- a | working | f.py | t")
                 + [asst("m2", [tool("t2", "Read", file_path="/w/TASKS.md")]), res("t2", "     1→- a | working | f.py | t")]
                 + bash("m3", "t3", "cat TASKS.md", "- a | working | f.py | t\n- b | working | g.py | u"))
        s = streams.parse_stream(lines, klatch_bin=KB)
        self.assertEqual(s["idle_turns"], 1)  # the Read of the unchanged board; the third read saw a new line

    def test_a_turn_with_a_read_and_an_edit_is_not_idle(self):
        lines = [asst("m1", [tool("t1", "Bash", command=KB + " read lobby"), tool("t2", "Edit", file_path="a.py")]),
                 res("t1", EMPTY_READ), res("t2", "ok")]
        self.assertEqual(streams.parse_stream(lines, klatch_bin=KB)["idle_turns"], 0)

    def test_a_turn_whose_result_never_arrived_is_not_called_idle(self):
        lines = [asst("m1", [tool("t1", "Bash", command="sleep 20")])]
        self.assertEqual(streams.parse_stream(lines, klatch_bin=KB)["idle_turns"], 0)

    def test_a_compound_read_command_is_idle_when_nothing_new(self):
        lines = (bash("m1", "t1", "git pull --no-rebase origin main && cat TASKS.md", "Already up to date.\n- a | working | f | t")
                 + bash("m2", "t2", "git pull --no-rebase origin main && cat TASKS.md", "Already up to date.\n- a | working | f | t"))
        self.assertEqual(streams.parse_stream(lines, klatch_bin=KB)["idle_turns"], 1)

    def test_a_command_that_is_not_a_read_is_never_idle(self):
        lines = bash("m1", "t1", "python3 -m unittest", "")
        self.assertEqual(streams.parse_stream(lines, klatch_bin=KB)["idle_turns"], 0)

    def test_an_idle_klatch_call_through_a_variable_counts_the_same(self):
        lines = bash("m1", "t1", 'K=%s; "$K" wait lobby --timeout 5' % KB, "timed out after 5 s")
        self.assertEqual(streams.parse_stream(lines, klatch_bin=KB)["idle_turns"], 1)


class ReadClassification(unittest.TestCase):
    def kinds(self, name, **inp):
        return cmdkinds.read_kinds(name, inp, KB)

    def test_per_arm_read_only_coordination_calls(self):
        for verb in ("read", "claims", "asks", "who", "wait", "digest", "board", "history"):
            self.assertIsNotNone(self.kinds("Bash", command="%s %s" % (KB, verb)), verb)
        for cmd in ("cat TASKS.md", "git pull --no-rebase origin main", "git fetch", "git log --oneline", "sleep 20",
                    "cd w && git pull"):
            self.assertIsNotNone(self.kinds("Bash", command=cmd), cmd)
        self.assertIsNotNone(self.kinds("ListAgents"))
        self.assertIsNotNone(self.kinds("Read", file_path="TASKS.md"))

    def test_writes_are_not_reads(self):
        for cmd in (KB + " post lobby x", KB + " claim f.py", "git push origin main", "echo x >> TASKS.md", "ls"):
            self.assertIsNone(self.kinds("Bash", command=cmd), cmd)
        self.assertIsNone(self.kinds("Read", file_path="a.py"))
        self.assertIsNone(self.kinds("SendMessage", to="b", message="x"))


if __name__ == "__main__":
    unittest.main()
