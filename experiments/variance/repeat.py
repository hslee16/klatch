#!/usr/bin/env python3
"""Repeat a scenario N times and report mean and spread per agent (#243).

A single run of a stochastic system says little: a difference of a few turns or cents between two variants may be
noise. This harness runs one scenario command N times, reads one JSON result from its stdout each time, and prints
mean, sample standard deviation, min and max for turns, tokens, cost and duration per agent, plus the count of each
outcome. Quote the spread next to any comparison.

    experiments/variance/repeat.py --runs 5 [--json FILE] -- COMMAND [ARGS...]

The scenario command is yours (a script that starts the agents, waits, and prints the result). It must print one JSON
object as the last line of stdout, in either form:

  * a `claude -p --output-format json` result: num_turns, total_cost_usd, duration_ms, is_error, usage{...}. It is
    read as one agent named "agent".
  * {"agents": {"<name>": {"turns", "tokens", "cost_usd", "duration_s", "outcome"}, ...}} for several agents; any
    field may be missing.

This script never starts an agent itself and spends nothing; the cost is whatever your scenario command costs, so run
real scenarios on demand (the human's choice), not in CI. Tests use a fake agent (test-repeat.sh).
"""
import argparse
import json
import math
import subprocess
import sys

METRICS = ["turns", "tokens", "cost_usd", "duration_s"]


def normalise_claude(obj):
    usage = obj.get("usage") or {}
    tokens = sum(
        usage.get(k, 0) or 0
        for k in ("input_tokens", "output_tokens", "cache_read_input_tokens", "cache_creation_input_tokens")
    )
    rec = {}
    if "num_turns" in obj:
        rec["turns"] = obj["num_turns"]
    if usage:
        rec["tokens"] = tokens
    if "total_cost_usd" in obj:
        rec["cost_usd"] = obj["total_cost_usd"]
    if "duration_ms" in obj:
        rec["duration_s"] = obj["duration_ms"] / 1000.0
    rec["outcome"] = obj.get("outcome") or ("error" if obj.get("is_error") else "ok")
    return {"agent": rec}


def parse_result(text):
    lines = [l for l in text.splitlines() if l.strip()]
    if not lines:
        raise ValueError("the scenario printed nothing")
    try:
        obj = json.loads(lines[-1])
    except json.JSONDecodeError as e:
        raise ValueError("the last line of stdout is not JSON: %s" % e)
    if not isinstance(obj, dict):
        raise ValueError("the result is not a JSON object")
    if isinstance(obj.get("agents"), dict):
        return {name: dict(rec) for name, rec in obj["agents"].items()}
    return normalise_claude(obj)


def stats(values):
    n = len(values)
    mean = sum(values) / n
    sd = math.sqrt(sum((v - mean) ** 2 for v in values) / (n - 1)) if n > 1 else None
    return {"values": values, "mean": mean, "sd": sd, "min": min(values), "max": max(values)}


def fmt(x):
    return ("%d" % x) if float(x).is_integer() else ("%g" % x)


def report(runs, results):
    agents = {}
    for per_run in results:
        for name, rec in per_run.items():
            a = agents.setdefault(name, {"outcomes": {}})
            for m in METRICS:
                if isinstance(rec.get(m), (int, float)):
                    a.setdefault(m, []).append(float(rec[m]))
            oc = str(rec.get("outcome", "unknown"))
            a["outcomes"][oc] = a["outcomes"].get(oc, 0) + 1
    out = {"runs": runs, "agents": {}}
    for name, a in agents.items():
        entry = {m: stats(a[m]) for m in METRICS if m in a}
        entry["outcomes"] = a["outcomes"]
        out["agents"][name] = entry
    return out


def print_report(rep):
    n = rep["runs"]
    print("runs: %d" % n)
    for name, a in rep["agents"].items():
        print("agent %s" % name)
        for m in METRICS:
            if m not in a:
                continue
            s = a[m]
            sd = "n/a" if s["sd"] is None else "%.2f" % s["sd"]
            print("  %-11s mean %.2f  sd %s  min %s  max %s" % (m, s["mean"], sd, fmt(s["min"]), fmt(s["max"])))
        oc = a["outcomes"]
        ok = oc.get("ok", 0)
        rest = "".join("  %s %d" % (k, v) for k, v in sorted(oc.items()) if k != "ok")
        print("  %-11s ok %d/%d%s" % ("outcome", ok, n, rest))


def main():
    argv = sys.argv[1:]
    cmd = []
    if "--" in argv:
        i = argv.index("--")
        argv, cmd = argv[:i], argv[i + 1:]
    p = argparse.ArgumentParser(description="Repeat a scenario N times and report variance (#243).")
    p.add_argument("--runs", type=int, default=5)
    p.add_argument("--json", help="write the raw per-run values and the statistics to this file")
    args = p.parse_args(argv)
    if args.runs < 1:
        p.error("--runs must be at least 1")
    if not cmd:
        p.error("give the scenario command after --")
    results = []
    for i in range(1, args.runs + 1):
        proc = subprocess.run(cmd, stdout=subprocess.PIPE, text=True)
        if proc.returncode != 0:
            print("run %d: the scenario exited %d" % (i, proc.returncode), file=sys.stderr)
            return 1
        try:
            results.append(parse_result(proc.stdout))
        except ValueError as e:
            print("run %d: %s" % (i, e), file=sys.stderr)
            return 1
    rep = report(args.runs, results)
    print_report(rep)
    if args.json:
        with open(args.json, "w") as f:
            json.dump(rep, f, indent=2)
    return 0


if __name__ == "__main__":
    sys.exit(main())
