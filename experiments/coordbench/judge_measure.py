#!/usr/bin/env python3
"""Scenario S10 part 2 (#1375): measure a screening judge on the judge-eval corpus, with no model.

Runs `klatch-bench judge-eval --judge rules|null --split S --corpus FILE --out TMP` once per split and turns each run's summary
into the two rates S10 asks for: the miss rate (an item that should not cross, a redact or deny item, was let through: the
runner's strict leak, over the items that should not cross) and the false positive rate (a clean item was redacted or denied:
the runner's over-block, over the clean items), each with a Wilson 95 percent interval. The record also carries the klatch commit
(and whether the tree was dirty), the corpus file's sha256 and item count and the bench binary's sha256, so a number can be
tied to the code and the data that made it. The numbers come from the run; nothing is copied from an earlier report.

This script spends nothing: a judge that calls a model (claude:..., jev..., cascade:...) is refused. The Sonnet and Opus arms of
S10 part 2 are described in scenarios/s10.json and need the human's budget approval; they are not run from here.

    judge_measure.py [--bench BIN] [--judge rules] --corpus FILE [--scrub FILE] [--splits dev,test] [--repo DIR] [--out FILE]

BIN defaults to $KLATCH_BENCH_BIN (a built klatch-bench: `cargo build --locked -p klatch-bench` in proto/).
"""
import argparse
import datetime
import hashlib
import json
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import stats  # noqa: E402

FREE_JUDGES = ("null", "rules")


class MeasureError(Exception):
    pass


def _ci(k, n):
    lo, hi = stats.wilson(k, n)
    return None if lo is None else [round(lo, 4), round(hi, 4)]


def summarise(result):
    """The measurement of one split from the runner's --out JSON ({"judge", "split", "summary"})."""
    s = result["summary"]
    rows = s["confusion"]["counts"]  # rows: truth allow, redact, deny; the columns are the judge's verdicts
    clean = sum(rows[0])
    should_not_cross = sum(rows[1]) + sum(rows[2])
    misses, fps = s["leaks"], s["overblocks"]
    cats = {c["category"]: {"n": c["n"], "misses": c["leaks"], "false_positives": c["overblocks"]}
            for c in s.get("by_category", []) if c["leaks"] or c["overblocks"]}
    return {"n": s["n"], "should_not_cross": should_not_cross, "clean": clean, "misses": misses,
            "miss_rate": misses / should_not_cross if should_not_cross else None,
            "miss_rate_ci95": _ci(misses, should_not_cross), "data_leaks": s.get("data_leaks"),
            "false_positives": fps, "false_positive_rate": fps / clean if clean else None,
            "false_positive_rate_ci95": _ci(fps, clean), "defers": s.get("defers", 0), "errors": s.get("errors", 0),
            "incomplete": bool(s.get("errors") or s.get("defers")), "contested_excluded": len(s.get("contested", [])),
            "by_category": cats}


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def _git(repo, *args):
    p = subprocess.run(["git", "-C", repo] + list(args), stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    if p.returncode != 0:
        raise MeasureError("git %s failed in %s" % (" ".join(args), repo))
    return p.stdout.strip()


def _corpus_entry(corpus, repo):
    top = _git(repo, "rev-parse", "--show-toplevel")
    inside = os.path.abspath(corpus).startswith(os.path.realpath(top))
    return {"path": os.path.relpath(os.path.abspath(corpus), top) if inside else os.path.basename(corpus),
            "sha256": sha256_file(corpus), "items": 0}


def _base_record(bench, judge, corpus, repo, now):
    return {"schema": 1, "kind": "judge-measurement", "scenario": "S10 part 2", "judge": judge,
            "date": now or datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "klatch_commit": _git(repo, "rev-parse", "HEAD"),
            "klatch_tree_dirty": bool(_git(repo, "status", "--porcelain", "--untracked-files=no")),
            "bench_binary_sha256": sha256_file(bench), "model_calls": 0, "cost_usd": 0.0,
            "corpus": _corpus_entry(corpus, repo), "splits": {}}


def _run_split(bench, judge, split, corpus, scrub):
    """(the split's summary, the judge-eval argv with <out> in place of the temporary output file)."""
    with tempfile.TemporaryDirectory(prefix="judge-measure-") as d:
        out = os.path.join(d, "out.json")
        argv = [bench, "judge-eval", "--judge", judge, "--split", split, "--corpus", corpus, "--out", out]
        if scrub:
            argv += ["--scrub", scrub]
        p = subprocess.run(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        if p.returncode != 0 or not os.path.exists(out):
            raise MeasureError("judge-eval failed on %s (exit %d): %s" % (split, p.returncode, p.stdout.strip()[-300:]))
        with open(out) as f:
            summary = summarise(json.load(f))
        return summary, "klatch-bench " + " ".join(argv[1:]).replace(out, "<out>")


def measure(bench, judge, corpus, scrub=None, splits=("dev", "test"), repo=None, now=None):
    """Run the free judge on each split and return the record (see the module docstring)."""
    if judge not in FREE_JUDGES:
        raise MeasureError("%r calls a model: this script only measures the free judges %s" % (judge, ", ".join(FREE_JUDGES)))
    record = _base_record(bench, judge, corpus, repo or HERE, now)
    with open(corpus) as f:
        record["corpus"]["items"] = sum(1 for l in f if l.strip())
    if scrub:
        record["scrub_sha256"] = sha256_file(scrub)
    for split in splits:
        record["splits"][split], command = _run_split(bench, judge, split, corpus, scrub)
        record.setdefault("command", command)
    return record


def line(split, m):
    pct = lambda r: "n/a" if r is None else "%.1f%%" % (100 * r)
    ci = lambda c: "" if c is None else " (95%% CI %.1f-%.1f%%)" % (100 * c[0], 100 * c[1])
    return "%s: miss %d/%d = %s%s, false positive %d/%d = %s%s%s" % (
        split, m["misses"], m["should_not_cross"], pct(m["miss_rate"]), ci(m["miss_rate_ci95"]), m["false_positives"],
        m["clean"], pct(m["false_positive_rate"]), ci(m["false_positive_rate_ci95"]),
        "  INCOMPLETE (errors or defers)" if m["incomplete"] else "")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--bench", default=os.environ.get("KLATCH_BENCH_BIN"))
    ap.add_argument("--judge", default="rules")
    ap.add_argument("--corpus", required=True)
    ap.add_argument("--scrub")
    ap.add_argument("--splits", default="dev,test")
    ap.add_argument("--repo")
    ap.add_argument("--out")
    a = ap.parse_args(argv)
    if not a.bench:
        print("judge_measure: give --bench or set KLATCH_BENCH_BIN (a built klatch-bench)", file=sys.stderr)
        return 2
    try:
        rec = measure(a.bench, a.judge, a.corpus, a.scrub, tuple(a.splits.split(",")), a.repo)
    except MeasureError as e:
        print("judge_measure: %s" % e, file=sys.stderr)
        return 1
    if a.out:
        with open(a.out, "w") as f:
            json.dump(rec, f, indent=1)
            f.write("\n")
    for split, m in rec["splits"].items():
        print(line(split, m))
    return 0


if __name__ == "__main__":
    sys.exit(main())
