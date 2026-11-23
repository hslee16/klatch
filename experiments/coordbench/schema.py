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


def count_units(checks):
    """(units_total, units_completed) from outcome_checks. A check counts as a unit unless it says "unit": false
    (structural checks such as "no conflict markers" are reported under correctness, not as units)."""
    units = [c for c in checks if isinstance(c, dict) and c.get("unit") is not False]
    return len(units), sum(1 for c in units if c.get("pass") is True)


def validate_record(rec):
    """A list of problems with the record (empty when it is valid)."""
    if not isinstance(rec, dict):
        return ["the record is not a JSON object"]
    errs = []
    for k in REQUIRED_KEYS:
        if k not in rec:
            errs.append("missing key %s" % k)
    for parent, keys in NESTED.items():
        sub = rec.get(parent)
        if parent in rec and not isinstance(sub, dict):
            errs.append("%s is not an object" % parent)
            continue
        for k in keys:
            if isinstance(sub, dict) and k not in sub:
                errs.append("missing key %s.%s" % (parent, k))
    if rec.get("schema") != SCHEMA_VERSION:
        errs.append("schema must be %d" % SCHEMA_VERSION)
    if rec.get("outcome") not in OUTCOMES:
        errs.append("outcome %r is not one of %s" % (rec.get("outcome"), ", ".join(OUTCOMES)))
    checks = rec.get("outcome_checks")
    if "outcome_checks" in rec and not isinstance(checks, list):
        errs.append("outcome_checks is not a list")
    elif isinstance(checks, list) and isinstance(rec.get("metrics"), dict) and "units_completed" in rec["metrics"]:
        passed = count_units(checks)[1]
        if rec["metrics"]["units_completed"] != passed:
            errs.append("metrics.units_completed (%s) is not the number of passing unit checks (%d)"
                        % (rec["metrics"]["units_completed"], passed))
    return errs
