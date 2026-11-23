"""The budget approval file the harness requires for real runs (--budget-approved-in FILE; SPEC section 8; #1399 item 7).

The human's approval is made at run time, by the coordinator, in a JSON file outside the repository. No approval file is
ever committed. The file is a single object with these fields, all required:

  approved_by        who approved (non-empty text)
  approved_on        the date of the approval, YYYY-MM-DD, not in the future
  suite_budget_usd   the most the whole suite may spend, a number above 0
  per_run_cap_usd    the most one run may spend, a number above 0 and not above suite_budget_usd
  scope              one sentence naming the suite or pilot the approval is for (non-empty text)
  models             the model ids that may be run (non-empty list of text); the agent model and the roles model
  expires_on         the last day the approval is valid, YYYY-MM-DD, not before approved_on

load() raises ApprovalError (the harness exits 2) for a missing or malformed file, an expired file, a model that is not
listed, or a cap given on the command line that is above the file's. Caps not given on the command line become the file's
numbers; caps given and not above the file's stand.
"""
import datetime
import json

FIELDS = ("approved_by", "approved_on", "suite_budget_usd", "per_run_cap_usd", "scope", "models", "expires_on")


class ApprovalError(Exception):
    pass


def _text(d, key):
    v = d.get(key)
    if not isinstance(v, str) or not v.strip():
        raise ApprovalError("budget approval: %s must be non-empty text" % key)
    return v.strip()


def _date(d, key):
    v = _text(d, key)
    try:
        if len(v) != 10:
            raise ValueError(v)
        return datetime.date.fromisoformat(v)
    except ValueError:
        raise ApprovalError("budget approval: %s must be a date as YYYY-MM-DD, got %r" % (key, v))


def _is_positive_number(v):
    return not isinstance(v, bool) and isinstance(v, (int, float)) and v > 0


def _usd(d, key):
    v = d.get(key)
    if not _is_positive_number(v):
        raise ApprovalError("budget approval: %s must be a number above 0" % key)
    return float(v)


def _read_object(path):
    try:
        with open(path) as f:
            d = json.load(f)
    except (OSError, ValueError) as e:
        raise ApprovalError("budget approval: cannot read %s as JSON (%s)" % (path, e.__class__.__name__))
    if not isinstance(d, dict):
        raise ApprovalError("budget approval: %s must hold one JSON object" % path)
    for k in FIELDS:
        if k not in d:
            raise ApprovalError("budget approval: field %s is missing" % k)
    return d


def _is_model_id(m):
    return isinstance(m, str) and bool(m.strip())


def _model_list(d):
    models = d["models"]
    if not (isinstance(models, list) and models):
        raise ApprovalError("budget approval: models must be a non-empty list of model ids")
    if not all(_is_model_id(m) for m in models):
        raise ApprovalError("budget approval: models must be a non-empty list of model ids")
    return models


def _check_window(approved, expires, today):
    if approved > today:
        raise ApprovalError("budget approval: approved_on %s is in the future" % approved)
    if expires < approved:
        raise ApprovalError("budget approval: expires_on %s is before approved_on %s" % (expires, approved))


def _check_expiry(expires, today):
    if today > expires:
        raise ApprovalError("budget approval: expired on %s (today is %s)" % (expires, today))


def _check_models(models, asked):
    for label, m in asked:
        if m and m not in models:
            raise ApprovalError("budget approval: the %s %s is not in the approved models (%s)" % (label, m, ", ".join(models)))


def _check_cli_caps(run_cap, suite_cap, per_run, suite):
    if run_cap is not None and run_cap > per_run:
        raise ApprovalError("budget approval: the per-run cap %g is above the approved %g" % (run_cap, per_run))
    if suite_cap is not None and suite_cap > suite:
        raise ApprovalError("budget approval: the suite cap %g is above the approved %g" % (suite_cap, suite))


def _effective(approved_cap, cli_cap):
    return min(approved_cap, cli_cap) if cli_cap is not None else approved_cap


def load(path, model, roles_model, run_cap, suite_cap, today=None):
    """Validate the file at path for a run of model (and roles_model) with the caps given on the command line (None when
    not given). Returns the approval with the effective caps as per_run_usd and suite_usd."""
    today = today or datetime.date.today()
    d = _read_object(path)
    out = {"approved_by": _text(d, "approved_by"), "scope": _text(d, "scope")}
    approved, expires = _date(d, "approved_on"), _date(d, "expires_on")
    suite, per_run = _usd(d, "suite_budget_usd"), _usd(d, "per_run_cap_usd")
    models = _model_list(d)
    _check_window(approved, expires, today)
    if per_run > suite:
        raise ApprovalError("budget approval: per_run_cap_usd %g is above suite_budget_usd %g" % (per_run, suite))
    _check_expiry(expires, today)
    _check_models(models, (("agent model", model), ("roles model", roles_model)))
    _check_cli_caps(run_cap, suite_cap, per_run, suite)
    out.update(approved_on=str(approved), expires_on=str(expires), models=list(models), suite_budget_usd=suite,
               per_run_cap_usd=per_run, per_run_usd=_effective(per_run, run_cap), suite_usd=_effective(suite, suite_cap))
    return out
