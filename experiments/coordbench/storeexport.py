"""The K arms' run store as counts (#1433): call `klatch-bench store-stats` on the run store and turn the report into metrics.

The tool is in the klatch-bench crate, not in the shipped klatch binary, and it prints exactly what `klatch admin stats
--json` prints for the same store (one library function builds both): counts, timestamps, room and member names, never a
message body, ask text, claim detail, label or payload. It refuses the live store and a key directory that is not mode 700.
The harness only ever hands it a run store it made itself (a temporary KLATCH_HOME and KLATCH_KEY_DIR).

A missing tool, an unreadable store or a bad answer is never an invalid run: the record keeps null and a note says why.
Standard library only.
"""
import json
import os
import subprocess

TOOL_NAME = "klatch-bench"


def find_tool(klatch_bin, env):
    """The bench tool: $KLATCH_BENCH_BIN, else `klatch-bench` beside the klatch binary (the same cargo target directory),
    else None."""
    given = (env or {}).get("KLATCH_BENCH_BIN")
    if given:
        return given
    if klatch_bin:
        beside = os.path.join(os.path.dirname(os.path.abspath(klatch_bin)), TOOL_NAME)
        if os.path.exists(beside):
            return beside
    return None


def _run_tool(argv, env, timeout):
    """(CompletedProcess, None), or (None, the note why the tool could not be run)."""
    try:
        p = subprocess.run(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=timeout, env=env)
    except (OSError, subprocess.SubprocessError) as e:
        return None, "store export failed: %s" % e
    return p, None


def _parse_report(stdout):
    try:
        report = json.loads(stdout)
    except ValueError:
        return None, "store export failed: the tool did not print JSON"
    if not isinstance(report, dict):
        return None, "store export failed: the tool did not print an object"
    return report, None


def export(tool, home, key_dir, env=None, timeout=120):
    """(report dict or None, note or None): the counts-only report of the run store at home, opened with the key in key_dir."""
    if not tool or not os.path.exists(tool):
        return None, "store export skipped: the klatch-bench tool was not found (%s; build it with cargo build -p klatch-bench)" % (
            tool or "no path")
    p, note = _run_tool([tool, "store-stats", "--home", home, "--key-dir", key_dir, "--json"], env, timeout)
    if p is None:
        return None, note
    if p.returncode != 0:
        return None, "store export failed: %s" % (p.stderr.strip()[-200:] or "exit %d" % p.returncode)
    return _parse_report(p.stdout)


def _sum(pairs, names=None):
    if not isinstance(pairs, list):
        return None
    return sum(int(p.get("count", 0)) for p in pairs if isinstance(p, dict) and (names is None or p.get("name") in names))


def _get(d, *path):
    for k in path:
        if not isinstance(d, dict) or k not in d:
            return None
        d = d[k]
    return d


def metrics(report):
    """The scenario metrics the store gives, each null when the report does not have the field (never zero by default).

    claim_conflicts: claims refused and later granted to the same member (the report's conflicts_handled: a grant that
    followed that member's own refusal). claims_refused_by_rule: every refusal, each of which carries a rule word (same-path,
    overlap, shared, deadlock; the report's `refused` list gives the split)."""
    hist = _get(report, "claims", "history") or {}
    refused = hist.get("refused")
    return {
        "claim_conflicts": hist.get("conflicts_handled"),
        "claims_granted": _sum(hist.get("granted")),
        "claims_refused_by_rule": _sum(refused),
        "claims_released": _sum(hist.get("released")),
        "claims_expired": _sum(hist.get("expired")),
        "asks_created": _get(report, "asks", "created"),
        "asks_answered": _get(report, "asks", "answered"),
        "asks_closed": _get(report, "asks", "closed"),
        "time_to_first_answer_s": _get(report, "asks", "first_answer_median_secs"),
        "members_posted_never_read": _get(report, "members", "posted_never_read"),
        "hook_bytes_injected": _sum(report.get("hook_bytes")),
    }
