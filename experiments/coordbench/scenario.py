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
