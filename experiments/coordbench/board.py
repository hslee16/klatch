"""B1's claim conflicts from the task board (#1524, SPEC section 5): the board equivalent of a klatch claim refusal.

The board TASKS.md has one line per piece of work, `- NAME | STATUS | FILES | TITLE`. A claim conflict is a file that two
different agents have a `working` line for in the same version of the board. Every version of TASKS.md that any commit of
the origin or an agent's clone has is read (git history of every ref), so a conflict that only exists after a merge, and one
that was seen by the second agent and ignored, are both found; a claim the first agent had already set to `done`, `blocked`
or `handoff` is no conflict (a later line of the same agent and title supersedes an earlier one, see _working_by_file). Each (file, pair of agents) counts once however many versions show it. Limits: the unit is a
file name as written in the FILES field (a directory claim does not conflict with a file under it); the order of claims is not
used, so a claim made while the other's was visible and one made blind are counted alike; a board edited through a
different path than TASKS.md at the repository root is not read. Standard library only; one `git` process per commit."""
import os
import subprocess

PATH = "TASKS.md"


def _parse_line(line):
    """The row of one board line, or None for a line that is not one."""
    line = line.strip()
    parts = [p.strip() for p in line[2:].split("|")] if line.startswith("- ") else []
    if len(parts) < 3 or not (parts[0] and parts[1]):
        return None
    return {"name": parts[0], "status": parts[1].lower(), "files": parts[2].replace(",", " ").split(),
            "title": " | ".join(parts[3:])}


def parse_board(text):
    """[{"name", "status", "files": [...], "title"}] for the board lines of `text`; any other line is skipped."""
    rows = [_parse_line(line) for line in (text or "").splitlines()]
    return [r for r in rows if r]


def _working_by_file(text):
    """{file: set of agents with a working line for it} in one version of the board. With merge=union (B1 repositories, #1523) a
    status change that meets another agent's append leaves the old line beside the new one, and two agents editing one line keep
    both versions; so the LAST line per (agent, title) in file order is the current state, and an earlier `working` line that a
    later `done`, `blocked` or `handoff` line of the same agent and title supersedes is not a claim."""
    last = {}
    for r in parse_board(text):
        last[(r["name"], r["title"])] = r
    claims = {}
    for r in last.values():
        for f in (r["files"] if r["status"] == "working" else []):
            claims.setdefault(f, set()).add(r["name"])
    return claims


def _pairs(claims):
    out = set()
    for f, agents in claims.items():
        ag = sorted(agents)
        out.update((f, a, b) for i, a in enumerate(ag) for b in ag[i + 1:])
    return out


def conflicts(versions):
    """[{"file", "agents": [a, b]}] over the board versions (texts): files two agents both have a `working` line for in one
    version, each (file, pair) once, in file then agent order."""
    seen = set()
    for text in versions:
        seen |= _pairs(_working_by_file(text))
    return [{"file": f, "agents": [a, b]} for f, a, b in sorted(seen)]


def _git(repo, args, env):
    p = subprocess.run(["git", "-C", repo] + args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
    return p.returncode, p.stdout.decode("utf-8", "replace")


def _blob_ids(repo, path, env):
    """The ids of every version of `path` in the history of every ref: the new side of each change in each commit, and of a
    merge against each parent (-m), so a union merge that differs from both parents is a version too."""
    if not os.path.isdir(repo):
        raise RuntimeError("no repository at %s" % repo)
    rc, out = _git(repo, ["log", "--all", "-m", "--format=", "--raw", "--no-abbrev", "--no-renames", "--", path], env)
    if rc != 0:
        raise RuntimeError("git log failed in %s" % repo)
    ids = {line.split()[3] for line in out.splitlines() if line.startswith(":") and len(line.split()) > 3}
    return sorted(i for i in ids if set(i) != {"0"})


def git_versions(repo, env, path=PATH):
    """{blob id: text} of every distinct version of `path` in the history of every ref of `repo`. Raises RuntimeError when
    the history cannot be read."""
    texts = {i: _git(repo, ["cat-file", "blob", i], env) for i in _blob_ids(repo, path, env)}
    return {i: text for i, (rc, text) in texts.items() if rc == 0}


def claim_conflicts(repos, env, path=PATH):
    """{"claim_conflicts": n, "detail": [...], "versions": number of distinct board versions read} over the board history of
    the origin and the clones."""
    versions = {}
    for repo in repos:
        versions.update(git_versions(repo, env, path))
    found = conflicts([versions[k] for k in sorted(versions)])
    return {"claim_conflicts": len(found), "detail": found, "versions": len(versions)}
