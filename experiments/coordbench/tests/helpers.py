"""Shared helpers for the coordbench tests: import path and a run record builder."""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
BENCH = os.path.dirname(HERE)
if BENCH not in sys.path:
    sys.path.insert(0, BENCH)


def make_record(scenario="trivial", arm="B0", run="r01", suite="s", units=(2, 2), cost=0.1, turns=10, wall=60.0,
                outcome="ok", valid=True, messages=0, tokens=1000):
    """A run record as the spec's schema (section 7.1) has it, with the fields the report reads filled in."""
    total, done = units
    return {
        "schema": 1, "run_id": "%s-%s-%s-%s" % (suite, scenario, arm.lower(), run), "milestone": suite,
        "scenario": scenario, "topology": "T-clone", "arm": arm, "arm_text_version": "draft-1",
        "fixture": {"name": scenario, "sha256": "0" * 64}, "seed": 1, "valid": valid, "supersedes": None,
        "models": {"agent": "fake", "roles": None, "resolved_ids": []}, "claude_code_version": None,
        "klatch": {"commit": "abc", "binary_sha256": None, "store_schema": None}, "temperature": "default",
        "caps": {"per_run_usd": 1.0, "suite_usd": 10.0, "wall_s": 100, "max_turns_per_agent": 30},
        "start": "2026-11-22T10:00:00Z", "end": "2026-11-22T10:01:00Z", "wall_s": wall, "time_to_first_pass_s": None,
        "outcome": outcome, "spend": {"usd": cost, "estimated": False},
        "turns": {"total": turns, "per_agent": {"a": turns}},
        "tokens": {"input": tokens, "output": 0, "cache_read": 0, "cache_creation": 0},
        "tool_calls": {"total": 3, "by_tool": {"Bash": 3}},
        "metrics": {"units_total": total, "units_completed": done, "duplicate_work": None, "git_conflicts": 0,
                    "claim_conflicts": None, "overwrites": None, "messages": messages, "human_interventions": 0,
                    "coordination_context_tokens": 0, "idle_turns": 0},
        "outcome_checks": [{"id": "u%d" % i, "pass": i < done} for i in range(total)],
        "artifacts": {"streams": None, "store_export": None, "checker_report": None}, "notes": "",
    }
