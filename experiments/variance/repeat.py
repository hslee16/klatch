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

Each run gets a fresh empty KLATCH_HOME and KLATCH_KEY_DIR (under a temp dir, removed afterwards; also
KLATCH_RUN_DIR for scratch), so no run touches ~/.klatch or the Keychain. --timeout SECONDS kills a hung run and its
children and counts it as a failure; --max-cost USD stops the loop once total cost reaches the cap (exit 3; it needs
cost in every result). A run that crashes, times out or prints no JSON is a failure, not an abort (exit 1 at the end).

This script never starts an agent itself and spends nothing; the cost is whatever your scenario command costs, so run
real scenarios on demand (the human's choice), not in CI. Tests use a fake agent (test-repeat.sh).
"""
import argparse
import json
import math
import os
import shutil
import signal
import subprocess
import sys
import tempfile

# adoption, lobby_messages, dm_messages and reply_latency_s come from scenario-specific results such as
# experiments/dm-adoption (#1194); an agent entry without them simply has no line for them.
METRICS = ["turns", "tokens", "cost_usd", "duration_s", "adoption", "lobby_messages", "dm_messages", "reply_latency_s"]


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
    failed = sum(1 for per_run in results if any(str(r.get("outcome", "unknown")) != "ok" for r in per_run.values()))
    out = {"runs": runs, "failures": failed, "agents": {}}
    for name, a in agents.items():
        entry = {m: stats(a[m]) for m in METRICS if m in a}
        entry["outcomes"] = a["outcomes"]
        out["agents"][name] = entry
    return out


def print_report(rep):
    n = rep["runs"]
    print("runs: %d" % n)
    print("failures: %d of %d" % (rep["failures"], n))
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


def run_once(cmd, timeout):
    """Run the scenario once in its own private stores. Returns (stdout, failure) where failure is None or an
    outcome word (crashed, timeout). The scenario runs in its own process group so a timeout kills its children."""
    base = tempfile.mkdtemp(prefix="klatch-repeat-")
    env = dict(os.environ)
    env["KLATCH_HOME"] = os.path.join(base, "home")
    env["KLATCH_KEY_DIR"] = os.path.join(base, "keys")
    os.makedirs(env["KLATCH_HOME"])
    os.makedirs(env["KLATCH_KEY_DIR"])
    env["KLATCH_RUN_DIR"] = base
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, text=True, env=env, start_new_session=True)
    try:
        out, _ = proc.communicate(timeout=timeout)
        failure = None if proc.returncode == 0 else "crashed"
        if failure:
            print("the scenario exited %d" % proc.returncode, file=sys.stderr)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except OSError:
            pass
        proc.communicate()
        out, failure = "", "timeout"
    finally:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except OSError:
            pass
        shutil.rmtree(base, ignore_errors=True)
    return out, failure


def main():
    argv = sys.argv[1:]
    cmd = []
    if "--" in argv:
        i = argv.index("--")
        argv, cmd = argv[:i], argv[i + 1:]
    p = argparse.ArgumentParser(description="Repeat a scenario N times and report variance (#243).")
    p.add_argument("--runs", type=int, default=5)
    p.add_argument("--json", help="write the raw per-run values and the statistics to this file")
    p.add_argument("--max-cost", type=float, help="hard total USD limit; stop starting runs once the sum reaches it")
    p.add_argument("--timeout", type=float, help="kill a run that takes longer than this many seconds")
    args = p.parse_args(argv)
    if args.runs < 1:
        p.error("--runs must be at least 1")
    if not cmd:
        p.error("give the scenario command after --")
    results = []
    harness_failures = 0
    spent = 0.0
    capped = False
    for i in range(1, args.runs + 1):
        out, failure = run_once(cmd, args.timeout)
        if failure is None:
            try:
                res = parse_result(out)
            except ValueError as e:
                print("run %d: %s" % (i, e), file=sys.stderr)
                failure = "unparseable"
        if failure is not None:
            harness_failures += 1
            results.append({"scenario": {"outcome": failure}})
            continue
        results.append(res)
        if args.max_cost is not None:
            costs = [r["cost_usd"] for r in res.values() if isinstance(r.get("cost_usd"), (int, float))]
            if not costs:
                print("run %d: no cost in the result, cannot enforce --max-cost" % i, file=sys.stderr)
                return 1
            spent += sum(costs)
            if spent >= args.max_cost and i < args.runs:
                capped = True
                break
    rep = report(len(results), results)
    print_report(rep)
    rep["spent_usd"] = spent
    if args.max_cost is not None:
        print("spent: %.2f of %.2f USD" % (spent, args.max_cost))
    if capped:
        print("stopped: spend cap %.2f USD reached after %d of %d runs" % (args.max_cost, len(results), args.runs))
    if args.json:
        with open(args.json, "w") as f:
            json.dump(rep, f, indent=2)
    if capped:
        return 3
    return 1 if harness_failures else 0


if __name__ == "__main__":
    sys.exit(main())
