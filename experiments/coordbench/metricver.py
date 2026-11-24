"""The metric version of a record (#1521, #1522, #1524) and what the report does about it.

METRIC_VERSION 2 changed what several metrics mean, so a record made before the change (no `metric_version` field: version 1)
and one made after must never be pooled in one cell:

  coordination_context_tokens   now counts inbound message events and coordination tool schemas as well as hook output and
                                coordination tool calls and results, in every arm (ctxcount.py); version 1 counted hook output
                                and tool results only, so B1 was undercounted
  idle_turns                    now a turn of read-only coordination calls whose results had no new content, with per-arm
                                equivalents (B1 reading TASKS.md, git pull, ListAgents); version 1 counted every klatch read,
                                claims, asks and who as idle and no B1 call
  messages                      now includes edits of the task board TASKS.md; version 1 counted SendMessage and klatch posts
  claim_conflicts               now computed for B1 from the board (board.py); version 1 left it null for a B arm
  S1 primary                    integration_ok (scenarios/s1.json); version 1 used units_completed

The report splits mixed versions into separate scenario groups (analyse), uses only the newest version for the scaling table, and
says so in its header; comparing two analyses of different versions is flagged."""

METRIC_VERSION = 2


def version_of(rec):
    v = rec.get("metric_version")
    return v if isinstance(v, int) and not isinstance(v, bool) else 1


def completion_basis(valid):
    """{"completion_basis": "integration_ok"} for a cell whose runs report the integration outcome (S1, #1522), else {}."""
    return {"completion_basis": "integration_ok"} if any(isinstance(r["metrics"].get("integration_ok"), bool) for r in valid) else {}


def condition(records):
    """{"metric version": [versions in order of appearance]} when any record carries the field, else {} (records made before
    metric version 2 have none, so a report of only those is unchanged)."""
    seen = []
    for r in records:
        if "metric_version" in r and r["metric_version"] not in seen:
            seen.append(r["metric_version"])
    return {"metric version": seen} if seen else {}


def split_by(records, version_fn, field, suffix):
    """(records for the per-scenario analysis, counts per version {"1": n, ...} or None). When the versions (version_fn of a
    record) are mixed an older record's scenario gets the suffix (`suffix % version`), so its cells are never pooled with the
    newest version's. When every record has one version the records come back unchanged and counts is None, unless a record
    carries the field explicitly. Used for the metric version and the fixture hash version (fixhash.py)."""
    counts = {}
    for r in records:
        counts[str(version_fn(r))] = counts.get(str(version_fn(r)), 0) + 1
    explicit = any(field in r for r in records)
    if len(counts) < 2:
        return records, (counts if explicit else None)
    newest = max(version_fn(r) for r in records)
    out = [r if version_fn(r) == newest else dict(r, scenario=r["scenario"] + suffix % version_fn(r)) for r in records]
    return out, counts


def newest_by(records, version_fn):
    """The records of the newest version (all of them when the versions are not mixed)."""
    newest = max([version_fn(r) for r in records] or [1])
    return [r for r in records if version_fn(r) == newest]


def split_scenarios(records):
    """split_by for the metric version: the older records' scenarios get ` [metric vN]`."""
    return split_by(records, version_of, "metric_version", " [metric v%d]")


def newest_only(records):
    """The records of the newest metric version (all of them when the versions are not mixed)."""
    return newest_by(records, version_of)


def header_lines(counts):
    """The header lines of a report whose records have mixed versions, else []."""
    if not counts or len(counts) < 2:
        return []
    parts = ", ".join("version %s: %d runs" % (v, counts[v]) for v in sorted(counts))
    return ["**Mixed metric versions** (%s). Records of different metric versions are not pooled: the older ones are listed as "
            "`SCENARIO [metric vN]` and the scaling table uses the newest only. See metricver.py for what changed." % parts, ""]


def mismatch(new, old):
    """A one-line note when two analyses were made from different metric versions, or None."""
    a, b = sorted((new.get("metric_versions") or {"1": 1}).keys()), sorted((old.get("metric_versions") or {"1": 1}).keys())
    return None if a == b else ("the analyses use different metric versions (now %s, before %s): the metrics are not comparable"
                                % (",".join(a), ",".join(b)))
