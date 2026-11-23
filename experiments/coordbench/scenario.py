"""Scenario files and the metrics that need the unit-to-path map (#1366, ticket #1399 item 1).

A scenario file (scenarios/<id>.json) names the fixture, arms, N per arm, caps and the metric definitions of one scenario,
and maps units to paths and agents to their units. A fixture points at its scenario with "scenario" in fixture.json. For a
fixture with a scenario the harness fills `duplicate_work` and `overwrites` from git history (git_metrics) and keeps the
checker's extra fields (merge state, per-agent contribution) under the record's `scenario_metrics`.
"""
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


def _git(repo, args, env):
    p = subprocess.run(["git", "-C", repo] + args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
    return p.returncode, p.stdout.decode("utf-8", "replace")


def _commits(repo, paths, env):
    """{sha: {"author", "files": {path: [added lines]}}} for every non-merge, non-root commit reachable from any ref."""
    rc, out = _git(repo, ["log", "--all", "--no-merges", "--no-renames", "-p", "--unified=0",
                          "--format=%x01%H%x01%an%x01%P%x01", "--"] + paths, env)
    if rc != 0:
        raise RuntimeError("git log failed in %s" % repo)
    parts = out.split("\x01")
    commits = {}
    for i in range(1, len(parts) - 3, 4):
        sha, author, parents, body = parts[i], parts[i + 1], parts[i + 2], parts[i + 3]
        if not parents.strip():
            continue  # the root commit is the harness's initial tree, not an agent's work
        files, cur, in_hunk = {}, None, False
        for line in body.split("\n"):
            if line.startswith("diff --git "):
                m = re.match(r"diff --git a/(.*) b/(.*)$", line)
                cur, in_hunk = (m.group(2) if m else None), False
                if cur is not None:
                    files.setdefault(cur, [])
            elif line.startswith("@@"):
                in_hunk = True
            elif in_hunk and cur is not None and line.startswith("+"):
                files[cur].append(line[1:])
        commits[sha] = {"author": author, "files": files}
    return commits


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
    authors = {u: set() for u in unit_path}
    added = {}  # (file, line) -> author of the first commit seen adding it, in a deterministic order
    for sha in sorted(commits):
        c = commits[sha]
        for f, lines in c["files"].items():
            for u, p in unit_path.items():
                if f == p:
                    authors[u].add(c["author"])
            if f in shared:
                pat = re.compile(shared[f]["entry"])
                for line in lines:
                    if pat.match(line.rstrip()):
                        added.setdefault((f, line.rstrip()), c["author"])
    final = {}
    for f in shared:
        rc, out = _git(origin, ["show", "%s:%s" % (branch, f)], env)
        final[f] = set(l.rstrip() for l in out.splitlines()) if rc == 0 else set()
    lost = [{"file": f, "line": line, "author": a} for (f, line), a in sorted(added.items()) if line not in final[f]]
    duplicated = [u for u in unit_path if len(authors[u]) > 1]
    foreign = [{"unit": u, "author": a, "owner": owner[u]} for u in unit_path for a in sorted(authors[u]) if a != owner[u]]
    return {"duplicate_work": len(duplicated), "overwrites": len(lost),
            "details": {"duplicated_units": duplicated, "foreign_edits": foreign, "lost_lines": lost}}


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


def history_overwrites(scn, origin, clones, env, branch="main"):
    """Edits committed anywhere and missing from the final main, by behaviour (scenario key history_check). Every non-merge,
    non-root commit reachable from any ref of the origin or any clone is tested with the hidden checker's per-ticket check on
    the file as that commit has it; a ticket that passes on some commit and fails on the final main is overwritten. Returns
    {"overwritten": [{"ticket", "author"}]} (the author of the first commit, in sha order, that had it)."""
    hc = scn["history_check"]
    check_py = os.path.join(HERE, "checks", hc["checker"], "check.py")
    seen, ever = set(), {}
    for repo in [origin] + list(clones):
        rc, out = _git(repo, ["log", "--all", "--no-merges", "--format=%x01%H%x01%an%x01%P%x01"], env)
        if rc != 0:
            raise RuntimeError("git log failed in %s" % repo)
        parts = out.split("\x01")
        for i in range(1, len(parts) - 3, 4):
            sha, author, parents = parts[i], parts[i + 1], parts[i + 2]
            if not parents.strip() or sha in seen:
                continue
            seen.add(sha)
            rc, text = _git(repo, ["show", "%s:%s" % (sha, hc["path"])], env)
            if rc != 0:
                continue
            for t, ok in sorted(_ticket_pass(check_py, text, env).items()):
                if ok and (t not in ever or sha < ever[t][0]):
                    ever[t] = (sha, author)
    rc, text = _git(origin, ["show", "%s:%s" % (branch, hc["path"])], env)
    final = _ticket_pass(check_py, text, env) if rc == 0 else {}
    return {"overwritten": [{"ticket": t, "author": ever[t][1]} for t in sorted(ever) if not final.get(t)]}


def overwrite_split(overwritten, git_conflicts, extras):
    """silent_overwrites, overwrites_after_conflict and conflicts_resolved of one run (see scenarios/s2.json)."""
    n = len(overwritten)
    conflicts = git_conflicts or 0
    return {"silent_overwrites": n if conflicts == 0 else 0,
            "overwrites_after_conflict": n if conflicts > 0 else 0,
            "conflicts_resolved": conflicts if (n == 0 and extras.get("correct") is True) else 0}
