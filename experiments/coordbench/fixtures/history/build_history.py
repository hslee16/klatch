#!/usr/bin/env python3
"""build_history.py DEST ARM: build the prebuilt repository of fixture F9 in DEST from history.json (#1374).

A git repository on branch main with one commit per entry of history.json, each by its own author at its own fixed time (one
hour apart from the start), so the commit ids are identical in every run and every arm of the same board kind. In arm B1 the
commit also appends its `board` lines to TASKS.md and its `decisions` lines to DECISIONS.md; in every other arm those lines are
ignored. A commit that changes no file is an empty commit (a decision or a handoff recorded in a message).
"""
import datetime
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))


def run_git(dest, base, args, env=None):
    p = subprocess.run(["git", "-C", dest] + list(args), env=env or base, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                       text=True)
    if p.returncode != 0:
        raise SystemExit("git %s failed: %s" % (" ".join(args), p.stdout.strip()[-300:]))


def write_board_headers(dest, hist):
    with open(os.path.join(dest, "TASKS.md"), "w") as f:
        f.write(hist["board_header"])
    with open(os.path.join(dest, "DECISIONS.md"), "w") as f:
        f.write(hist["decisions_header"])


def write_commit_files(dest, commit, board):
    """The files of one commit, and in arm B1 its board and decisions lines appended to TASKS.md and DECISIONS.md."""
    for path, text in commit["files"].items():
        os.makedirs(os.path.dirname(os.path.join(dest, path)) or dest, exist_ok=True)
        with open(os.path.join(dest, path), "w") as f:
            f.write(text)
    if not board:
        return
    for name, lines in (("TASKS.md", commit["board"]), ("DECISIONS.md", commit["decisions"])):
        if lines:
            with open(os.path.join(dest, name), "a") as f:
                f.write("".join(l + "\n" for l in lines))


def commit_env(base, c, when):
    return dict(base, GIT_AUTHOR_NAME=c["author"], GIT_AUTHOR_EMAIL=c["author"] + "@bench.invalid", GIT_AUTHOR_DATE=when,
                GIT_COMMITTER_NAME=c["author"], GIT_COMMITTER_EMAIL=c["author"] + "@bench.invalid", GIT_COMMITTER_DATE=when)


def main(dest, arm):
    with open(os.path.join(HERE, "history.json")) as f:
        hist = json.load(f)
    base = {k: os.environ[k] for k in ("PATH", "LANG", "LC_ALL") if k in os.environ}
    base.update(HOME=dest, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull, GIT_TERMINAL_PROMPT="0")
    run_git(dest, base, ["init", "-q", "-b", "main"])
    t0 = datetime.datetime.strptime(hist["start"], "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=datetime.timezone.utc)
    board = arm == "B1"
    if board:
        write_board_headers(dest, hist)
    for i, c in enumerate(hist["commits"]):
        write_commit_files(dest, c, board)
        when = (t0 + datetime.timedelta(hours=i)).strftime("%Y-%m-%dT%H:%M:%S+0000")
        env = commit_env(base, c, when)
        run_git(dest, base, ["add", "-A"], env)
        run_git(dest, base, ["commit", "-q", "--allow-empty", "-m", c["message"]], env)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("usage: build_history.py DEST ARM")
    main(sys.argv[1], sys.argv[2])
