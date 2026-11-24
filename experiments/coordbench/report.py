#!/usr/bin/env python3
"""Generate REPORT.md from a results directory, and compare it with a previous release (#1364, SPEC sections 6 and 7).

    report.py RESULTS_DIR [--baseline OLD_RESULTS_DIR] [--out FILE] [--json FILE] [--primary METRIC] [--check-ledger]

Reads every run record (results/<suite>/<scenario>/<arm>/<run-id>.json). Per scenario and arm it tabulates N, the
completion rate with a Wilson interval, the outcome counts and, for each metric, the median, quartiles, range, mean and
sd. Between arms (each K arm against each B arm) it runs the tests the spec names: Fisher exact on the completion rate
(difference with a Newcombe interval and the risk ratio) and Mann-Whitney U on each continuous metric (Hodges-Lehmann
shift and a seeded bootstrap interval of the difference in medians). The Holm correction covers the primary-metric tests
of the whole report; other metrics carry no significance claim. A scenario with fewer than 5 runs in any compared arm is
labelled EXPLORATORY and claims nothing. With --baseline it also compares each scenario and arm with the previous
directory and flags regressions: a median worse than the previous median by more than the previous interquartile range,
confirmed only when the Holm-corrected test agrees and N is at least 5.

Runs with valid false are counted and not analysed; every other outcome (timeout, spend-cap, crash, error) stays in N and
is scored on what the hidden checker found. Standard library only.
"""
import argparse
import collections
import datetime
import json
import os
import sys

import fixhash
import metricver
import msglabel
import scale
import stats

DEFAULT_SEED = 20261122
DEFAULT_RESAMPLES = 10000
MIN_N = 5
INVALID_FLAG_SHARE = 0.2
CENSORING_OUTCOMES = ("timeout",)

# name -> (getter, higher_is_better)
CONT = {
    "units_completed": (lambda r: r["metrics"].get("units_completed"), True),
    "cost_usd": (lambda r: r["spend"].get("usd"), False),
    "turns": (lambda r: r["turns"].get("total"), False),
    # the cost of the role members of K3 and B1-roles (#1421), already inside cost_usd; absent for an arm without roles
    "roles_cost_usd": (lambda r: r["spend"].get("roles_usd"), None),
    "wall_s": (lambda r: r.get("wall_s"), False),
    "tokens": (lambda r: sum(v for v in r["tokens"].values() if isinstance(v, (int, float))), False),
    "messages": (lambda r: r["metrics"].get("messages"), None),
    "git_conflicts": (lambda r: r["metrics"].get("git_conflicts"), False),
    "coordination_context_tokens": (lambda r: r["metrics"].get("coordination_context_tokens"), False),
    "idle_turns": (lambda r: r["metrics"].get("idle_turns"), False),
    "human_interventions": (lambda r: r["metrics"].get("human_interventions"), False),
}


def _store(name):
    """A getter of one store metric (scenario_metrics.store, from the K arms' run store export, #1433); None for a B arm."""
    return lambda r: ((r.get("scenario_metrics") or {}).get("store") or {}).get(name)


# the store metrics show in the per-metric table for the arms that have them; never tested, never compared
CONT.update({
    "claim_conflicts": (lambda r: r["metrics"].get("claim_conflicts"), False),
    "claims_granted": (_store("claims_granted"), None),
    "claims_refused_by_rule": (_store("claims_refused_by_rule"), None),
    "claims_released": (_store("claims_released"), None),
    "claims_expired": (_store("claims_expired"), None),
    "asks_created": (_store("asks_created"), None),
    "asks_answered": (_store("asks_answered"), None),
    "asks_closed": (_store("asks_closed"), None),
    "time_to_first_answer_s": (_store("time_to_first_answer_s"), False),
    "members_posted_never_read": (_store("members_posted_never_read"), False),
    "hook_bytes_injected": (_store("hook_bytes_injected"), None),
})
TESTED = ("units_completed", "cost_usd", "turns", "wall_s", "tokens", "messages", "coordination_context_tokens")
COMPARED = ("units_completed", "cost_usd", "turns", "wall_s")
REQUIRED = ("run_id", "scenario", "arm", "metrics", "spend", "turns", "tokens")


def _read_record(p):
    """(record, problem): the run record in the file p, or (None, why it is not one)."""
    try:
        with open(p) as f:
            obj = json.load(f)
    except (OSError, ValueError) as e:
        return None, "%s: %s" % (p, e)
    if not isinstance(obj, dict) or any(k not in obj for k in REQUIRED):
        return None, "%s: not a run record" % p
    return obj, None


def load_results(root):
    """(records, problems): every JSON file under root that is a run record. suite.json and other files are skipped;
    a file that is not valid JSON or lacks the record keys is reported in problems, not dropped silently."""
    recs, problems = [], []
    for d, dirs, files in os.walk(root):
        dirs.sort()
        for n in sorted(files):
            if not n.endswith(".json") or n == "suite.json":
                continue
            obj, problem = _read_record(os.path.join(d, n))
            if problem:
                problems.append(problem)
            else:
                recs.append(obj)
    return recs, problems


def is_complete(r):
    ok = r["metrics"].get("integration_ok")
    if isinstance(ok, bool):  # S1 (#1522): the scenario's primary outcome is integration, not the module count
        return ok
    total = r["metrics"].get("units_total") or 0
    return total > 0 and r["metrics"].get("units_completed") == total


def _is_number(v):
    return isinstance(v, (int, float)) and not isinstance(v, bool)


def _metric_values(valid):
    return {name: [float(v) for v in (get(r) for r in valid) if _is_number(v)] for name, (get, _) in CONT.items()}


def _outcome_counts(valid):
    outcomes = {}
    for r in valid:
        outcomes[r["outcome"]] = outcomes.get(r["outcome"], 0) + 1
    return outcomes


def cell_of(runs, all_runs):
    valid = [r for r in runs if r.get("valid", True)]
    invalid = len(runs) - len(valid)
    n = len(valid)
    k = sum(1 for r in valid if is_complete(r))
    values = _metric_values(valid)
    lo, hi = stats.wilson(k, n)
    cell = {
        "n": n, "n_invalid": invalid, "invalid_flag": bool(runs) and invalid / len(runs) > INVALID_FLAG_SHARE,
        "completed": k, "completion_rate": (k / n) if n else None, "wilson": [lo, hi], "outcomes": _outcome_counts(valid),
        "censored_wall": sum(1 for r in valid if r["outcome"] in CENSORING_OUTCOMES),
        "metrics": {name: stats.summary(v) for name, v in values.items()}, "values": values,
        "valid_runs": [{"outcome": r["outcome"], "wall_s": r.get("wall_s"), "complete": is_complete(r)} for r in valid],
    }
    return dict(cell, **metricver.completion_basis(valid), **msglabel.cell_fields(runs))


def pairs_of(arms):
    bs = [a for a in arms if a.startswith("B")]
    ks = [a for a in arms if a.startswith("K")]
    return [(k, b) for k in ks for b in bs] if bs and ks else _later_vs_earlier(arms)


def _later_vs_earlier(arms):
    return [(arms[j], arms[i]) for i in range(len(arms)) for j in range(i + 1, len(arms))]


def test_continuous(metric, x, y, ax, ay, primary, exploratory, seed, resamples, cx=None, cy=None):
    """Mann-Whitney U of arm x against arm y (y is the reference): the shift is x minus y."""
    mw = stats.mann_whitney(y, x)  # symmetric in p; groups are named for the shift below
    lo, hi = stats.bootstrap_median_diff_ci(y, x, resamples=resamples, seed=seed)
    t = {"arm": ax, "against": ay, "metric": metric, "primary": primary, "method": "Mann-Whitney U",
         "u_method": mw["method"], "p": mw["p"], "p_holm": None, "n_arm": len(x), "n_against": len(y),
         "shift": stats.hodges_lehmann(y, x), "shift_ci": [lo, hi], "exploratory": exploratory,
         "significant": False, "p_without_censored": None}
    return t


Opts = collections.namedtuple("Opts", "primary seed resamples")


def _is_small(cx, cy):
    return min(cx["n"], cy["n"]) < MIN_N


def _completion_test(ax, ay, cx, cy):
    d, dlo, dhi = stats.newcombe_diff(cx["completed"], cx["n"], cy["completed"], cy["n"])
    return {"arm": ax, "against": ay, "metric": "completion", "primary": True, "method": "Fisher exact",
            "p": stats.fisher_exact(cx["completed"], cx["n"] - cx["completed"], cy["completed"],
                                    cy["n"] - cy["completed"]), "p_holm": None, "n_arm": cx["n"],
            "n_against": cy["n"], "diff": d, "diff_ci": [dlo, dhi],
            "rr": stats.risk_ratio(cx["completed"], cx["n"], cy["completed"], cy["n"]), "exploratory": _is_small(cx, cy),
            "significant": False}


def _uncensored_walls(runs):
    return [float(r["wall_s"]) for r in runs if r.get("valid", True) and r["outcome"] not in CENSORING_OUTCOMES
            and isinstance(r.get("wall_s"), (int, float))]


def _p_without_censored(runs_x, runs_y):
    xs, ys = _uncensored_walls(runs_x), _uncensored_walls(runs_y)
    return stats.mann_whitney(ys, xs)["p"] if xs and ys else None


def _pair_tests(runs, pair, cells, opts):
    """The tests of arm ax against arm ay of one scenario: the completion rate, then each tested continuous metric."""
    ax, ay = pair
    cx, cy = cells[ax], cells[ay]
    expl = _is_small(cx, cy)
    tests = [_completion_test(ax, ay, cx, cy)]
    for m in TESTED:
        x, y = cx["values"][m], cy["values"][m]
        if not x or not y:
            continue
        tc = test_continuous(m, x, y, ax, ay, m == opts.primary, expl, opts.seed, opts.resamples)
        if m == "wall_s":
            tc["p_without_censored"] = _p_without_censored(runs[ax], runs[ay])
        tests.append(tc)
    return tests


def _scenario_tests(runs, arms, cells, opts):
    tests = []
    for ax, ay in pairs_of(arms):
        if not cells[ax]["n"] or not cells[ay]["n"]:
            continue
        tests += _pair_tests(runs, (ax, ay), cells, opts)
    return tests


def _scenario_entry(cells, tests):
    return {"arms": cells, "tests": tests,
            "exploratory": any(c["n"] < MIN_N for c in cells.values() if c["n"] or c["n_invalid"])}


def _holm_adjust(primary_tests):
    adj = stats.holm([t["p"] for t in primary_tests])
    for t, a in zip(primary_tests, adj):
        t["p_holm"] = a
        t["significant"] = (not t["exploratory"]) and a < 0.05


def _by_scenario_and_arm(records):
    by = {}
    for r in records:
        by.setdefault(r["scenario"], {}).setdefault(r["arm"], []).append(r)
    return by


def analyse(records, primary="units_completed", seed=DEFAULT_SEED, resamples=DEFAULT_RESAMPLES):
    newest = fixhash.newest_only(metricver.newest_only(records))  # the scaling table never mixes metric or hash versions
    records, versions = metricver.split_scenarios(records)
    records, hash_versions = fixhash.split_scenarios(records)
    by = _by_scenario_and_arm(records)
    out = {"records": len(records), "invalid_runs": sum(1 for r in records if not r.get("valid", True)),
           "primary_metric": primary, "bootstrap_seed": seed, "bootstrap_resamples": resamples, "scenarios": {},
           "conditions": conditions(records)}
    primary_tests = []
    total = 0
    for sc in sorted(by):
        arms = sorted(by[sc])
        cells = {a: cell_of(by[sc][a], records) for a in arms}
        tests = _scenario_tests(by[sc], arms, cells, Opts(primary, seed, resamples))
        total += len(tests)
        primary_tests += [t for t in tests if t["primary"]]
        out["scenarios"][sc] = _scenario_entry(cells, tests)
    _holm_adjust(primary_tests)
    out["tests_made"] = total
    out["primary_tests_made"] = len(primary_tests)
    out["scaling"] = scale.scaling(newest)
    if versions:
        out["metric_versions"] = versions
    if hash_versions:
        out["fixture_hash_versions"] = hash_versions
    return out


def conditions(records):
    def uniq(get):
        vals = []
        for r in records:
            try:
                v = get(r)
            except (KeyError, TypeError):
                continue
            if v is not None and v not in vals:
                vals.append(v)
        return vals
    out = {
        "milestones": uniq(lambda r: r["milestone"]),
        "agent models": uniq(lambda r: r["models"]["agent"]),
        "resolved model ids": sorted({i for r in records for i in (r.get("models") or {}).get("resolved_ids", [])}),
        "Claude Code version": uniq(lambda r: r["claude_code_version"]),
        "klatch commit": uniq(lambda r: r["klatch"]["commit"]),
        "klatch binary sha256": uniq(lambda r: r["klatch"]["binary_sha256"]),
        "arm text version": uniq(lambda r: r["arm_text_version"]),
        "fixture sha256": uniq(lambda r: r["fixture"]["sha256"]),
        "temperature": uniq(lambda r: r["temperature"]),
        "caps": uniq(lambda r: json.dumps(r["caps"], sort_keys=True)),
    }
    return dict(out, **metricver.condition(records), **fixhash.condition(records))


def _completion_row(key, cn, co):
    sc, arm = key
    p = stats.fisher_exact(cn["completed"], cn["n"] - cn["completed"], co["completed"], co["n"] - co["completed"])
    worse = (cn["completion_rate"] or 0) < (co["completion_rate"] or 0)
    return {"scenario": sc, "arm": arm, "metric": "completion", "old_median": co["completion_rate"],
            "old_q1": None, "old_q3": None, "new_median": cn["completion_rate"],
            "change": (cn["completion_rate"] or 0) - (co["completion_rate"] or 0), "n_old": co["n"],
            "n_new": cn["n"], "p": p, "p_holm": None, "worse": worse, "small": _is_small(cn, co), "flag": ""}, p


def _metric_row(key, m, cn, co):
    sc, arm = key
    so, sn = co["metrics"][m], cn["metrics"][m]
    if not so or not sn:
        return None, None
    iqr = so["q3"] - so["q1"]
    worse = sn["median"] < so["median"] - iqr if CONT[m][1] else sn["median"] > so["median"] + iqr
    p = stats.mann_whitney(co["values"][m], cn["values"][m])["p"]
    return {"scenario": sc, "arm": arm, "metric": m, "old_median": so["median"], "old_q1": so["q1"],
            "old_q3": so["q3"], "new_median": sn["median"], "change": sn["median"] - so["median"],
            "n_old": co["n"], "n_new": cn["n"], "p": p, "p_holm": None, "worse": worse,
            "small": _is_small(cn, co), "flag": ""}, p


def _cell_rows(key, cn, co):
    """[(row, p)] of one scenario and arm present in both analyses: the completion row, then each compared metric."""
    pairs = [_completion_row(key, cn, co)] + [_metric_row(key, m, cn, co) for m in COMPARED]
    return [(dict(row, **msglabel.carry(cn)), p) for row, p in pairs if row is not None]


def _shared_cells(new, old):
    """(scenario, arm, new cell, old cell) for every scenario and arm both analyses have runs for."""
    for sc in sorted(new["scenarios"]):
        old_arms = old["scenarios"].get(sc, {}).get("arms", {})
        for arm in sorted(new["scenarios"][sc]["arms"]):
            cn, co = new["scenarios"][sc]["arms"][arm], old_arms.get(arm)
            if co is not None and min(cn["n"], co["n"]) > 0:
                yield sc, arm, cn, co


def compare(new, old, resamples=DEFAULT_RESAMPLES, seed=DEFAULT_SEED):
    """Compare two analyses cell by cell (scenario and arm present in both). Returns {"rows": [...]}."""
    pairs = []
    for sc, arm, cn, co in _shared_cells(new, old):
        pairs += _cell_rows((sc, arm), cn, co)
    rows = [row for row, _ in pairs]
    for row, a in zip(rows, stats.holm([p for _, p in pairs])):
        row["p_holm"] = a
        if row["worse"]:
            row["flag"] = "REGRESSION" if (a < 0.05 and not row["small"]) else "possible regression (not confirmed)"
    return dict({"rows": rows, "tests_made": len(rows)}, **fixhash.version_notes(new, old))


def _ledger_ids(path):
    if not os.path.exists(path):
        return []
    with open(path) as f:
        return [l.split("|")[0].strip() for l in f.read().splitlines() if l.strip() and not l.startswith("#")]


def check_ledger(root):
    """Compare the ledger with the record files: the line count must equal the number of records (SPEC 7.2)."""
    recs, _ = load_results(root)
    ids = sorted(r["run_id"] for r in recs)
    path = os.path.join(root, "LEDGER.md")
    if not os.path.exists(path):
        path = os.path.join(os.path.dirname(os.path.abspath(root)), "LEDGER.md")
    lines = _ledger_ids(path)
    info = {"ledger": path, "records": len(ids), "ledger_lines": len(lines),
            "missing_from_ledger": sorted(set(ids) - set(lines)), "missing_records": sorted(set(lines) - set(ids))}
    return len(ids) == len(lines) and not info["missing_from_ledger"] and not info["missing_records"], info


# rendering --------------------------------------------------------------------------------------------------------

def _is_whole(x):
    return isinstance(x, float) and x.is_integer() and abs(x) < 1e9


def f(x, nd=2):
    if x is None:
        return "n/a"
    if _is_whole(x):
        return "%d" % x
    return ("%." + str(nd) + "g") % x if abs(x) >= 1000 else ("%." + str(nd) + "f") % x


def pf(p):
    return "n/a" if p is None else ("%.4f" % p if p >= 0.0001 else "%.1e" % p)


def iv(lo_hi, nd=2):
    return "[%s, %s]" % (f(lo_hi[0], nd), f(lo_hi[1], nd)) if lo_hi and lo_hi[0] is not None else "n/a"


def table(header, rows):
    out = ["| " + " | ".join(header) + " |", "|" + "|".join("---" for _ in header) + "|"]
    out += ["| " + " | ".join(str(c) for c in r) + " |" for r in rows]
    return "\n".join(out)


def _header_lines(a, source):
    return ["# Coordination bench report", "",
            "Generated %s%s. %d run records, %d invalid (kept, not analysed)." % (
                datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d %H:%M UTC"),
                " from %s" % source if source else "", a["records"], a["invalid_runs"]), ""]


def _conditions_lines(a):
    rows = [(k, ", ".join(str(x) for x in v) or "n/a") for k, v in a["conditions"].items()]
    return ["## Conditions", "", table(["Condition", "Value"], rows), ""]


def _flag_label(r):
    return r["flag"].upper() if r["flag"] == "REGRESSION" else r["flag"]


def _flagged_lines(cmp_, flagged):
    if not flagged:
        return ["No regressions flagged (%d comparisons)." % cmp_["tests_made"]]
    intro = ("Flags (%d of %d comparisons). A regression is a median worse than the previous median by more than the "
             "previous interquartile range; it is confirmed only when the Holm-corrected test agrees and N is at least "
             "%d in both. The human decides what a regression means for a release." % (len(flagged), cmp_["tests_made"], MIN_N))
    rows = [(_flag_label(r), r["scenario"], msglabel.label(r["arm"], r), r["metric"],
             "%s %s" % (f(r["old_median"]), iv([r["old_q1"], r["old_q3"]]) if r["old_q1"] is not None else ""),
             f(r["new_median"]), f(r["change"]), "%d/%d" % (r["n_old"], r["n_new"]), pf(r["p"]), pf(r["p_holm"]))
            for r in flagged]
    return [intro, "", table(["Flag", "Scenario", "Arm", "Metric", "Previous median [IQR]", "Now", "Change", "N old/new", "p",
                              "p Holm"], rows)]


def _comparison_lines(cmp_, baseline_dir):
    flagged = [r for r in cmp_["rows"] if r["flag"]]
    lines = ["## Comparison with %s" % baseline_dir, ""]
    lines += fixhash.comparison_lines(cmp_)
    lines += _flagged_lines(cmp_, flagged)
    rows = [(r["scenario"], msglabel.label(r["arm"], r), r["metric"], f(r["old_median"]), f(r["new_median"]), f(r["change"]),
             "%d/%d" % (r["n_old"], r["n_new"]), pf(r["p_holm"]), r["flag"]) for r in cmp_["rows"]]
    lines += ["", "All compared cells:", "",
              table(["Scenario", "Arm", "Metric", "Previous median", "Now", "Change", "N old/new", "p Holm", "Flag"], rows), ""]
    return lines


def _median_of(m, key, nd=2):
    return f(m[key]["median"], nd) if m[key] else "n/a"


def _arm_row(arm, c):
    m = c["metrics"]
    oc = ", ".join("%s %d" % (k, v) for k, v in sorted(c["outcomes"].items()))
    return (msglabel.label(arm, c), c["n"], "%d%s" % (c["n_invalid"], " (FLAG: over 20 percent)" if c["invalid_flag"] else ""),
            "%d of %d (%s)" % (c["completed"], c["n"], f(c["completion_rate"]) if c["n"] else "n/a"),
            iv(c["wilson"]), oc or "none", _median_of(m, "units_completed"), _median_of(m, "cost_usd", 3),
            _median_of(m, "turns"),
            "%s (%d censored)" % (f(m["wall_s"]["median"]), c["censored_wall"]) if m["wall_s"] else "n/a")


def _metric_rows(s):
    return [(msglabel.label(arm, c), name, sm["n"], f(sm["median"], 3), f(sm["q1"], 3), f(sm["q3"], 3), f(sm["min"], 3), f(sm["max"], 3),
             f(sm["mean"], 3), f(sm["sd"], 3))
            for arm, c in s["arms"].items() for name, sm in c["metrics"].items() if sm]


def _effect_text(t):
    if t["method"] == "Fisher exact":
        return "diff %s %s, risk ratio %s" % (f(t["diff"]), iv(t["diff_ci"]), f(t["rr"]))
    eff = "shift %s %s" % (f(t["shift"], 3), iv(t["shift_ci"], 3))
    if t["metric"] == "wall_s" and t["p_without_censored"] is not None:
        eff += "; p without censored %s" % pf(t["p_without_censored"])
    return eff


def _reading_label(t):
    if t["exploratory"]:
        return "EXPLORATORY, no claim"
    if t["significant"]:
        return "significant"
    return "not significant" if t["primary"] else "no claim (not primary)"


def _test_row(t):
    return ("%s vs %s" % (t["arm"], t["against"]), t["metric"],
            (t["method"] + (" (%s)" % t["u_method"] if "u_method" in t else "")),
            "%d/%d" % (t["n_arm"], t["n_against"]), pf(t["p"]), pf(t["p_holm"]), _effect_text(t), _reading_label(t))


READING_NOTE = ("Reading the tests: with 10 runs per arm, Fisher exact detects 0.9 against 0.2 with power 0.84 but 0.9 "
                "against 0.4 with only 0.47 (SPEC 6.1), and Mann-Whitney detects a shift of about 1.3 standard deviations "
                "with power 0.78. A result of no significant difference is not the same as no difference; effects under about "
                "one standard deviation are out of reach at affordable N. Means and standard deviations are a second line.")


def _tests_lines(tests):
    if not tests:
        return []
    return ["Tests, each K arm against each B arm. The shift is the first arm minus the second.", "",
            table(["Pair", "Metric", "Test", "N", "p", "p Holm", "Effect", "Reading"], [_test_row(t) for t in tests]),
            "", READING_NOTE, ""]


def _scenario_lines(sc, s):
    lines = ["## Scenario %s" % sc, ""]
    if s["exploratory"]:
        lines += ["**EXPLORATORY**: at least one arm has fewer than %d runs. The tables are the result; no significance claim "
                  "is made." % MIN_N, ""]
    basis = "integration ok" if any(c.get("completion_basis") for c in s["arms"].values()) else "all units"
    lines.append(table(["Arm", "N", "invalid", "Completed (%s)" % basis, "Wilson 95%", "Outcomes", "Units median",
                        "Cost USD median", "Turns median", "Wall s median"],
                       [_arm_row(arm, c) for arm, c in s["arms"].items()]))
    lines += ["", "Per metric, median with quartiles, range, then mean and sd as a second line:", ""]
    lines.append(table(["Arm", "Metric", "N", "median", "q25", "q75", "min", "max", "mean", "sd"], _metric_rows(s)))
    lines.append("")
    return lines + _tests_lines(msglabel.relabel_tests(s["tests"], s["arms"]))


def _tests_made_line(a):
    return ("Tests made: %d (primary-metric tests %d, Holm-corrected across the whole report, primary metric `%s` plus the "
            "completion rate). Bootstrap: %d resamples, seed %d." % (a["tests_made"], a["primary_tests_made"],
                                                                     a["primary_metric"], a["bootstrap_resamples"],
                                                                     a["bootstrap_seed"]))


def render_markdown(a, cmp_=None, baseline_dir=None, source=None, comparison_only=False):
    L = _header_lines(a, source) + msglabel.note_lines(a)
    if not a["scenarios"]:
        return "\n".join(L + ["No runs were found.", ""])
    if not comparison_only:
        L += _conditions_lines(a)
        L += fixhash.mixed_version_lines(a)
    if cmp_ is not None:
        L += _comparison_lines(cmp_, baseline_dir)
    if comparison_only:
        return "\n".join(L)
    L += [_tests_made_line(a), ""]
    for sc, s in a["scenarios"].items():
        L += _scenario_lines(sc, s)
    L += scale.render(a.get("scaling"))
    return "\n".join(L)


def _run_ledger_check(results_dir):
    ok, info = check_ledger(results_dir)
    print("ledger %s: %d lines, %d record files%s" % (info["ledger"], info["ledger_lines"], info["records"],
                                                      "" if ok else " (MISMATCH)"))
    for k in ("missing_from_ledger", "missing_records"):
        if info[k]:
            print("  %s: %s" % (k, ", ".join(info[k])))
    return 0 if ok else 1


def _load_with_warnings(directory):
    recs, problems = load_results(directory)
    for pr in problems:
        print("warning: %s" % pr, file=sys.stderr)
    return recs


def _write_json(path, analysis):
    with open(path, "w") as fh:
        json.dump(analysis, fh, indent=2, sort_keys=True)
        fh.write("\n")


def _deliver(a, md, an):
    if a.comparison_only or a.stdout:
        print(md)
        return
    out = a.out or os.path.join(a.results_dir, "REPORT.md")
    with open(out, "w") as fh:
        fh.write(md + "\n")
    print("wrote %s (%d scenarios, %d tests made)" % (out, len(an["scenarios"]), an["tests_made"]))


def _parse(argv):
    p = argparse.ArgumentParser(description="Generate REPORT.md from a results directory (#1364).")
    p.add_argument("results_dir")
    p.add_argument("--baseline", help="a previous results directory to compare with")
    p.add_argument("--out", help="the report file (default RESULTS_DIR/REPORT.md)")
    p.add_argument("--json", help="also write the analysis as JSON to this file")
    p.add_argument("--primary", default="units_completed", choices=sorted(CONT))
    p.add_argument("--seed", type=int, default=DEFAULT_SEED, help="bootstrap seed")
    p.add_argument("--resamples", type=int, default=DEFAULT_RESAMPLES)
    p.add_argument("--check-ledger", action="store_true", help="only check that LEDGER.md matches the record files")
    p.add_argument("--comparison-only", action="store_true", help="print only the comparison (needs --baseline)")
    p.add_argument("--stdout", action="store_true", help="print the report instead of writing it")
    return p.parse_args(argv)


def main(argv=None):
    a = _parse(argv)
    if a.check_ledger:
        return _run_ledger_check(a.results_dir)
    an = analyse(_load_with_warnings(a.results_dir), a.primary, a.seed, a.resamples)
    cmp_ = None
    if a.baseline:
        cmp_ = compare(an, analyse(_load_with_warnings(a.baseline), a.primary, a.seed, a.resamples))
        an["comparison"] = cmp_
    md = render_markdown(an, cmp_, a.baseline, a.results_dir, comparison_only=a.comparison_only)
    if a.json:
        _write_json(a.json, an)
    _deliver(a, md, an)
    return 0


if __name__ == "__main__":
    sys.exit(main())
