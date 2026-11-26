#!/usr/bin/env python3
"""build_history.py DEST ARM [SEED]: build the prebuilt repository of fixture F9 in DEST from history.json (#1374).

A git repository on branch main with one commit per entry of history.json, each by its own author at its own fixed time (one
hour apart from the start), so the commit ids are identical in every run and every arm of the same board kind. In arm B1 the
commit also appends its `board` lines to TASKS.md and its `decisions` lines to DECISIONS.md; in every other arm those lines are
ignored. A commit that changes no file is an empty commit (a decision or a handoff recorded in a message).

SEED (#1531) is the run's seed: the facts in the commit messages are drawn from it (gen_facts.generate), so the commit ids differ
from seed to seed and are the same for the same seed and arm. Without SEED, or with the generator's default seed, the committed
history.json is used.
"""
import datetime
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))


class Repo:
    """The repository under construction: git runs with a clean environment (no user or system configuration)."""

    def __init__(self, dest):
        self.dest = dest
        self.base = {k: os.environ[k] for k in ("PATH", "LANG", "LC_ALL") if k in os.environ}
        self.base.update(HOME=dest, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull, GIT_TERMINAL_PROMPT="0")

    def git(self, *args, env=None):
        p = subprocess.run(["git", "-C", self.dest] + list(args), env=env or self.base, stdout=subprocess.PIPE,
                           stderr=subprocess.STDOUT, text=True)
        if p.returncode != 0:
            raise SystemExit("git %s failed: %s" % (" ".join(args), p.stdout.strip()[-300:]))

    def write(self, path, text, mode="w"):
        full = os.path.join(self.dest, path)
        os.makedirs(os.path.dirname(full) or self.dest, exist_ok=True)
        with open(full, mode) as f:
            f.write(text)

    def add_board_attributes(self):
        """Append the union rule to .gitattributes (create the file when there is none), once (#1523)."""
        path = os.path.join(self.dest, ".gitattributes")
        have = open(path).read() if os.path.exists(path) else ""
        if "TASKS.md merge=union" not in have.split("\n"):
            self.write(".gitattributes", ("" if have == "" or have.endswith("\n") else "\n") + "TASKS.md merge=union\n", mode="a")

    def author_env(self, author, when):
        return dict(self.base, GIT_AUTHOR_NAME=author, GIT_AUTHOR_EMAIL=author + "@bench.invalid", GIT_AUTHOR_DATE=when,
                    GIT_COMMITTER_NAME=author, GIT_COMMITTER_EMAIL=author + "@bench.invalid", GIT_COMMITTER_DATE=when)


def _commit(repo, c, when, board):
    for path, text in c["files"].items():
        repo.write(path, text)
    if board:
        for name, lines in (("TASKS.md", c["board"]), ("DECISIONS.md", c["decisions"])):
            if lines:
                repo.write(name, "".join(l + "\n" for l in lines), mode="a")
    env = repo.author_env(c["author"], when)
    repo.git("add", "-A", env=env)
    repo.git("commit", "-q", "--allow-empty", "-m", c["message"], env=env)


def load_history(seed):
    """The scripted history of the seed: the committed history.json for no seed or the default seed, else drawn from the seed."""
    if seed is not None:
        if HERE not in sys.path:
            sys.path.insert(0, HERE)
        import gen_facts
        if seed != gen_facts.SEED:
            return json.loads(gen_facts.generate(seed)["fixtures/history/history.json"])
    with open(os.path.join(HERE, "history.json")) as f:
        return json.load(f)


def main(dest, arm, seed=None):
    hist = load_history(seed)
    repo = Repo(dest)
    repo.git("init", "-q", "-b", "main")
    t0 = datetime.datetime.strptime(hist["start"], "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=datetime.timezone.utc)
    board = arm == "B1"
    if board:
        repo.write("TASKS.md", hist["board_header"])
        repo.add_board_attributes()  # two appends to the board merge without a conflict (#1523)
        repo.write("DECISIONS.md", hist["decisions_header"])
    for i, c in enumerate(hist["commits"]):
        _commit(repo, c, (t0 + datetime.timedelta(hours=i)).strftime("%Y-%m-%dT%H:%M:%S+0000"), board)


if __name__ == "__main__":
    if len(sys.argv) not in (3, 4):
        raise SystemExit("usage: build_history.py DEST ARM [SEED]")
    main(sys.argv[1], sys.argv[2], int(sys.argv[3]) if len(sys.argv) == 4 else None)
