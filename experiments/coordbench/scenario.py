"""Scenario files and the metrics that need the unit-to-path map (#1366, ticket #1399 item 1).

A scenario file (scenarios/<id>.json) names the fixture, arms, N per arm, caps and the metric definitions of one scenario,
and maps units to paths and agents to their units. A fixture points at its scenario with "scenario" in fixture.json. For a
fixture with a scenario the harness fills `duplicate_work` and `overwrites` from git history (git_metrics) and keeps the
checker's extra fields (merge state, per-agent contribution) under the record's `scenario_metrics`.
"""
import collections
import json
import os
import re
import subprocess

HERE = os.path.dirname(os.path.abspath(__file__))


class ScenarioError(Exception):
    pass


def load(scenario_id, scenarios_dir=None):
    path = os.path.join(scenarios_dir or os.path.join(HERE, "scenarios"), scenario_id + ".json")
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError) as e:
        raise ScenarioError("cannot load scenario %s: %s" % (scenario_id, e))


def _fixture_scenario(name, fixtures_dir):
    """The scenario file id a fixture names, or None."""
    try:
        with open(os.path.join(fixtures_dir, name, "fixture.json")) as f:
            return json.load(f).get("scenario")
    except (OSError, ValueError, AttributeError):
        return None


def primary_metric(name, scenarios_dir=None, fixtures_dir=None):
    """The primary metric a scenario file names (metrics.primary) for the scenario `name` of a run record, or None.

    A record's scenario is its fixture's name, with "-<variant>" after it for a variant and " [metric vN]" after it for an
    older metric version (report.py); the fixture's own file says which scenario file it belongs to (#1670)."""
    fixtures_dir = fixtures_dir or os.path.join(HERE, "fixtures")
    parts = name.split(" [")[0].split("-")
    for i in range(len(parts), 0, -1):  # the whole name first, then without the last "-part"
        scn = _fixture_scenario("-".join(parts[:i]), fixtures_dir)
        if scn:
            try:
                metrics = load(scn, scenarios_dir).get("metrics")
            except ScenarioError:
                return None
            primary = metrics.get("primary") if isinstance(metrics, dict) else None
            return primary if isinstance(primary, str) else None
    return None


def _git(repo, args, env):
    p = subprocess.run(["git", "-C", repo] + args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
    return p.returncode, p.stdout.decode("utf-8", "replace")


def _is_added_line(line, cur, in_hunk):
    return in_hunk and cur is not None and line.startswith("+")


def _added_lines_by_file(body):
    """{path: [added lines]} of one commit's unified-0 diff text."""
    files, cur, in_hunk = {}, None, False
    for line in body.split("\n"):
        if line.startswith("diff --git "):
            m = re.match(r"diff --git a/(.*) b/(.*)$", line)
            cur, in_hunk = (m.group(2) if m else None), False
            if cur is not None:
                files.setdefault(cur, [])
        elif line.startswith("@@"):
            in_hunk = True
        elif _is_added_line(line, cur, in_hunk):
            files[cur].append(line[1:])
    return files


def _log_records(out):
    """[(sha, author, parents, body)] of the git log output with the \\x01-separated format."""
    parts = out.split("\x01")
    return [tuple(parts[i:i + 4]) for i in range(1, len(parts) - 3, 4)]


def _commits(repo, paths, env):
    """{sha: {"author", "files": {path: [added lines]}}} for every non-merge, non-root commit reachable from any ref.
    --full-history: a path-limited log otherwise drops a side branch whose merge equals the other side (#1471)."""
    rc, out = _git(repo, ["log", "--all", "--no-merges", "--full-history", "--no-renames", "-p", "--unified=0",
                          "--format=%x01%H%x01%an%x01%P%x01", "--"] + paths, env)
    if rc != 0:
        raise RuntimeError("git log failed in %s" % repo)
    return {sha: {"author": author, "files": _added_lines_by_file(body)}
            for sha, author, parents, body in _log_records(out)
            if parents.strip()}  # the root commit is the harness's initial tree, not an agent's work


def _shared_additions(c, shared):
    """[((file, line), author)] of the lines this commit added that match a shared file's entry pattern."""
    out = []
    for f, lines in c["files"].items():
        if f in shared:
            pat = re.compile(shared[f]["entry"])
            out += [((f, line.rstrip()), c["author"]) for line in lines if pat.match(line.rstrip())]
    return out


def _scan_commits(commits, unit_path, shared):
    """(authors per unit, {(file, line): author of the first commit seen adding it}) over the commits in sha order."""
    authors = {u: set() for u in unit_path}
    added = {}
    for sha in sorted(commits):
        c = commits[sha]
        for u, p in unit_path.items():
            if p in c["files"]:
                authors[u].add(c["author"])
        for key, author in _shared_additions(c, shared):
            added.setdefault(key, author)
    return authors, added


def _final_lines(origin, shared, branch, env):
    final = {}
    for f in shared:
        rc, out = _git(origin, ["show", "%s:%s" % (branch, f)], env)
        final[f] = set(l.rstrip() for l in out.splitlines()) if rc == 0 else set()
    return final


def _lost_lines(added, final):
    return [{"file": f, "line": line, "author": a} for (f, line), a in sorted(added.items()) if line not in final[f]]


def _foreign_edits(authors, owner):
    """Edits to a unit by anyone but the agent that owns it; authors is {unit: set of authors}, in unit order."""
    return [{"unit": u, "author": a, "owner": owner[u]} for u in authors for a in sorted(authors[u]) if a != owner[u]]


def git_metrics(scn, origin, clones, env, branch="main"):
    """duplicate_work, overwrites and their detail from git history. origin is the bare origin, clones the agents' clones
    (their local commits count, as the spec asks). Returns {"duplicate_work", "overwrites", "details"}."""
    unit_path = {u: v["path"] for u, v in scn["units"].items()}
    owner = {u: a for a, v in scn["agents"].items() for u in v["units"]}
    shared = scn["shared_files"]
    paths = sorted(set(unit_path.values()) | set(shared))
    commits = {}
    for repo in [origin] + list(clones):
        commits.update(_commits(repo, paths, env))
    authors, added = _scan_commits(commits, unit_path, shared)
    final = _final_lines(origin, shared, branch, env)
    lost = _lost_lines(added, final)
    duplicated = [u for u in unit_path if len(authors[u]) > 1]
    foreign = _foreign_edits(authors, owner)
    return {"duplicate_work": len(duplicated), "overwrites": len(lost),
            "details": {"duplicated_units": duplicated, "foreign_edits": foreign, "lost_lines": lost}}


def integration_ok(scn, checks):
    """The integration outcome of one run (#1522): True when every check the scenario lists under integration_checks passed,
    False when one failed, None when the scenario lists none or the checker did not report one of them (never False for a
    check that is missing). For S1 these are the structural checks (registry, changelog, no conflict markers, visible tests),
    none of which is a module's hidden behaviour test: units_completed carries those."""
    ids = scn.get("integration_checks")
    by = {c.get("id"): c.get("pass") for c in checks if isinstance(c, dict)}
    if not ids or any(i not in by for i in ids):
        return None
    return all(by[i] is True for i in ids)


def checker_extras(raw):
    """The fields the checker printed besides "checks" (merge_state, correct, per_agent, ...), or {}."""
    try:
        obj = json.loads(raw.strip().splitlines()[-1])
    except (ValueError, IndexError):
        return {}
    if not isinstance(obj, dict):
        return {}
    return {k: v for k, v in obj.items() if k not in ("checks", "units_completed", "units_total")}


def _ticket_pass(check_py, text, env):
    """{ticket: bool} for one version of the file, by the hidden checker's --file mode ({} when it cannot run)."""
    import sys
    import tempfile
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "core.py")
        with open(path, "w") as f:
            f.write(text)
        try:
            p = subprocess.run([sys.executable, "-I", check_py, "--file", path], stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL, env=env, timeout=60)
            obj = json.loads(p.stdout.decode("utf-8", "replace").strip().splitlines()[-1])
        except (subprocess.TimeoutExpired, ValueError, IndexError, OSError):
            return {}
    return obj if isinstance(obj, dict) else {}


TicketCheck = collections.namedtuple("TicketCheck", "path check_py env")


def _log_commits(repo, env):
    """[(sha, author)] of the non-merge, non-root commits reachable from any ref of the repository."""
    rc, out = _git(repo, ["log", "--all", "--no-merges", "--format=%x01%H%x01%an%x01%P%x01"], env)
    if rc != 0:
        raise RuntimeError("git log failed in %s" % repo)
    return [(sha, author) for sha, author, parents, _ in _log_records(out) if parents.strip()]


def _is_earlier(ever, ticket, sha):
    return ticket not in ever or sha < ever[ticket][0]


def _first_commits_passing(repo, tc, seen, ever):
    """Test every commit of the repository not seen yet with the per-ticket check; ever[ticket] is the (sha, author) of
    the first commit in sha order that had it."""
    for sha, author in _log_commits(repo, tc.env):
        if sha in seen:
            continue
        seen.add(sha)
        rc, text = _git(repo, ["show", "%s:%s" % (sha, tc.path)], tc.env)
        if rc != 0:
            continue
        for t, ok in sorted(_ticket_pass(tc.check_py, text, tc.env).items()):
            if ok and _is_earlier(ever, t, sha):
                ever[t] = (sha, author)


def history_overwrites(scn, origin, clones, env, branch="main"):
    """Edits committed anywhere and missing from the final main, by behaviour (scenario key history_check). Every non-merge,
    non-root commit reachable from any ref of the origin or any clone is tested with the hidden checker's per-ticket check on
    the file as that commit has it; a ticket that passes on some commit and fails on the final main is overwritten. Returns
    {"overwritten": [{"ticket", "author"}]} (the author of the first commit, in sha order, that had it)."""
    hc = scn["history_check"]
    tc = TicketCheck(hc["path"], os.path.join(HERE, "checks", hc["checker"], "check.py"), env)
    seen, ever = set(), {}
    for repo in [origin] + list(clones):
        _first_commits_passing(repo, tc, seen, ever)
    rc, text = _git(origin, ["show", "%s:%s" % (branch, hc["path"])], env)
    final = _ticket_pass(tc.check_py, text, env) if rc == 0 else {}
    return {"overwritten": [{"ticket": t, "author": ever[t][1]} for t in sorted(ever) if not final.get(t)]}


def overwrite_split(overwritten, git_conflicts, extras):
    """silent_overwrites, overwrites_after_conflict and conflicts_resolved of one run (see scenarios/s2.json)."""
    n = len(overwritten)
    conflicts = git_conflicts or 0
    return {"silent_overwrites": n if conflicts == 0 else 0,
            "overwrites_after_conflict": n if conflicts > 0 else 0,
            "conflicts_resolved": conflicts if (n == 0 and extras.get("correct") is True) else 0}
