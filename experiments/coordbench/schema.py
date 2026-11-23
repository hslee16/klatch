"""The per-run record schema of SPEC section 7.1 and its validation. Fields that do not apply are null, never omitted."""

SCHEMA_VERSION = 1
OUTCOMES = ("ok", "timeout", "spend-cap", "crash", "error")
REQUIRED_KEYS = ("schema", "run_id", "milestone", "scenario", "topology", "arm", "arm_text_version", "fixture", "seed",
                 "valid", "supersedes", "models", "claude_code_version", "klatch", "temperature", "caps", "start",
                 "end", "wall_s", "time_to_first_pass_s", "outcome", "spend", "turns", "tokens", "tool_calls",
                 "metrics", "outcome_checks", "artifacts", "notes")
NESTED = {
    "fixture": ("name", "sha256"),
    "models": ("agent", "roles", "resolved_ids"),
    "klatch": ("commit", "binary_sha256", "store_schema"),
    "caps": ("per_run_usd", "suite_usd", "wall_s", "max_turns_per_agent"),
    "spend": ("usd", "estimated"),
    "turns": ("total", "per_agent"),
    "tokens": ("input", "output", "cache_read", "cache_creation"),
    "tool_calls": ("total", "by_tool"),
    "metrics": ("units_total", "units_completed", "duplicate_work", "git_conflicts", "claim_conflicts", "overwrites",
                "messages", "human_interventions", "coordination_context_tokens", "idle_turns"),
    "artifacts": ("streams", "store_export", "checker_report"),
}


ROLE_MEMBER_KEYS = ("turns", "tokens", "cost_usd", "wall_s", "exit", "state")
ROLE_STATES = ("ok", "failed", "stopped-by-harness", "killed-by-cap")


def _has_bad_state(m):
    return isinstance(m, dict) and "state" in m and m["state"] not in ROLE_STATES


def _member_errors(group, name, m):
    errs = ["missing key roles.%s.%s.%s" % (group, name, k) for k in ROLE_MEMBER_KEYS if not isinstance(m, dict) or k not in m]
    if _has_bad_state(m):
        errs.append("roles.%s.%s.state %r is not one of %s" % (group, name, m["state"], ", ".join(ROLE_STATES)))
    return errs


def validate_roles(roles):
    """Problems with the optional `roles` object (#1421): null for an arm without roles, else per member the numbers a role
    must report (turns, tokens, cost, wall time, exit and state). The roles' cost is also in spend.usd (the team total)."""
    if roles is None:
        return []
    if not isinstance(roles, dict) or not isinstance(roles.get("members"), dict):
        return ["roles is not null or an object with members"]
    groups = [("members", roles["members"])]
    if isinstance(roles.get("replacements"), dict):
        groups.append(("replacements", roles["replacements"]))
    return [e for group, members in groups for name, m in members.items() for e in _member_errors(group, name, m)]


def count_units(checks):
    """(units_total, units_completed) from outcome_checks. A check counts as a unit unless it says "unit": false
    (structural checks such as "no conflict markers" are reported under correctness, not as units)."""
    units = [c for c in checks if isinstance(c, dict) and c.get("unit") is not False]
    return len(units), sum(1 for c in units if c.get("pass") is True)


def _nested_errors(rec):
    errs = []
    for parent, keys in NESTED.items():
        sub = rec.get(parent)
        if parent in rec and not isinstance(sub, dict):
            errs.append("%s is not an object" % parent)
        elif isinstance(sub, dict):
            errs += ["missing key %s.%s" % (parent, k) for k in keys if k not in sub]
    return errs


def _reports_units(metrics):
    return isinstance(metrics, dict) and "units_completed" in metrics


def _checks_errors(rec):
    checks = rec.get("outcome_checks")
    if "outcome_checks" in rec and not isinstance(checks, list):
        return ["outcome_checks is not a list"]
    metrics = rec.get("metrics")
    if not isinstance(checks, list) or not _reports_units(metrics):
        return []
    passed = count_units(checks)[1]
    if metrics["units_completed"] != passed:
        return ["metrics.units_completed (%s) is not the number of passing unit checks (%d)" % (metrics["units_completed"], passed)]
    return []


def validate_record(rec):
    """A list of problems with the record (empty when it is valid)."""
    if not isinstance(rec, dict):
        return ["the record is not a JSON object"]
    errs = ["missing key %s" % k for k in REQUIRED_KEYS if k not in rec]
    errs += _nested_errors(rec)
    if rec.get("schema") != SCHEMA_VERSION:
        errs.append("schema must be %d" % SCHEMA_VERSION)
    if rec.get("outcome") not in OUTCOMES:
        errs.append("outcome %r is not one of %s" % (rec.get("outcome"), ", ".join(OUTCOMES)))
    return errs + validate_roles(rec.get("roles")) + _checks_errors(rec)
