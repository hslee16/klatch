"""The per-run record schema of SPEC section 7.1 and its validation. Fields that do not apply are null, never omitted."""

SCHEMA_VERSION = 1
OUTCOMES = ("ok", "timeout", "spend-cap", "turn-cap", "crash", "error", "trigger-not-fired")
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


MESSAGING_KEYS = ("tools_present", "listagents_named_peer", "messages_sent", "messages_delivered")
ISOLATION_KEYS = ("mode", "checked_calls", "violation_count", "violations")


def _isolation_errors(iso):
    """The isolation object (#1519) is optional (null when the scan did not run); when present it has its four fields."""
    if iso is None:
        return []
    if not isinstance(iso, dict):
        return ["isolation must be null or an object"]
    errs = ["isolation misses %s" % k for k in ISOLATION_KEYS if k not in iso]
    if isinstance(iso.get("violations"), list) is False and "violations" in iso:
        errs.append("isolation.violations must be a list")
    return errs


def _messaging_errors(mv):
    """messaging_verified (#1520) is optional: null for an arm without messaging, else the object messaging.judge builds."""
    if mv is None:
        return []
    if not isinstance(mv, dict):
        return ["messaging_verified must be null or an object"]
    return ["messaging_verified misses %s" % k for k in MESSAGING_KEYS if k not in mv]


def _is_hex(v, n):
    return isinstance(v, str) and len(v) == n and all(c in "0123456789abcdef" for c in v)


def _identity_errors(rec):
    """The run identity fields added after schema 1 (#1530, #1547) are optional (a record made before has none), but when
    present they must have their shape: arm_text_sha256 a sha256 or null, prompt_sha256 null or {agent: sha256}, klatch.version_output
    a string or null, klatch.bench_commit a string or null."""
    errs = []
    if "arm_text_sha256" in rec and rec["arm_text_sha256"] is not None and not _is_hex(rec["arm_text_sha256"], 64):
        errs.append("arm_text_sha256 is not a sha256 or null")
    ps = rec.get("prompt_sha256")
    if ps is not None and not (isinstance(ps, dict) and all(_is_hex(v, 64) for v in ps.values())):
        errs.append("prompt_sha256 is not null or an object of sha256 values")
    k = rec.get("klatch")
    if isinstance(k, dict):
        errs += ["klatch.%s is not a string or null" % key for key in ("version_output", "bench_commit")
                 if k.get(key) is not None and not isinstance(k[key], str)]
    return errs


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
    return errs + _identity_errors(rec) + validate_roles(rec.get("roles")) + _checks_errors(rec) + _isolation_errors(rec.get("isolation")) + _messaging_errors(rec.get("messaging_verified"))
