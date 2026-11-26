"""The harness's own helper processes and guards (split out of run.py, #1455): the clock, hashes, the live-directory guard,
the minimal environment of git and the checker, killing a process group, and the facts about the klatch binary.
Standard library only; nothing here reads a credential."""
import datetime
import hashlib
import os
import re
import signal
import subprocess

import drivers


EXCLUDE = "\n.claude/\n"


def append_exclude(clone, text=EXCLUDE):
    with open(os.path.join(clone, ".git", "info", "exclude"), "a") as f:
        f.write(text)


class GuardError(Exception):
    pass


def now_iso():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def inside(path, root):
    path, root = os.path.realpath(path), os.path.realpath(root)
    return path == root or path.startswith(root.rstrip(os.sep) + os.sep)


def _check_store_path(p, live, tmp_root):
    for l in live:
        if inside(p, l):
            raise GuardError("refusing the live directory: %s resolves under %s" % (p, l))
    if not inside(p, tmp_root):
        raise GuardError("%s resolves outside the temp root %s" % (p, tmp_root))


def guard_paths(klatch_home, key_dir, tmp_root):
    """Refuse any store path that could be the live one (SPEC 8, Isolation). Raises GuardError."""
    home = os.path.expanduser("~")
    live = [os.path.join(home, ".klatch"), os.path.join(home, ".claude")]
    for l in live:
        if inside(tmp_root, l):
            raise GuardError("the temp root %s is under %s" % (tmp_root, l))
    for p in (klatch_home, key_dir):
        _check_store_path(p, live, tmp_root)


def minimal_env(base, scratch_home):
    """The environment of the harness's own helper processes (git, setup, checker, klatch --version): an allowlist,
    a private HOME, never a credential."""
    env = {k: base[k] for k in drivers.BASE_ENV_KEYS if k in base}
    env.update({"HOME": scratch_home, "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
                "GIT_TERMINAL_PROMPT": "0", "GIT_AUTHOR_NAME": "bench", "GIT_AUTHOR_EMAIL": "bench@bench.invalid",
                "GIT_COMMITTER_NAME": "bench", "GIT_COMMITTER_EMAIL": "bench@bench.invalid"})
    return env


def git(args, cwd, env):
    p = subprocess.run(["git"] + args, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    if p.returncode != 0:
        raise RuntimeError("git %s failed: %s" % (" ".join(args), p.stdout.strip()[-300:]))
    return p.stdout


def kill_group(proc):
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except OSError:
        pass


def klatch_facts(klatch_bin, env):
    """(binary sha256, store schema, `--version` output) of the klatch under test; all None when no binary was given. The
    binary's own identity: its version line does not name a commit, so none is read from it (#1547)."""
    if not klatch_bin:
        return None, None, None
    schema_no, version = None, None
    try:
        p = subprocess.run([klatch_bin, "--version"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
                           env=env, timeout=30)
        m = re.search(r"store schema (\d+)", p.stdout)
        schema_no = int(m.group(1)) if m else None
        version = p.stdout.strip() if p.returncode == 0 and p.stdout.strip() else None
    except (OSError, subprocess.SubprocessError):
        pass
    return sha256_file(klatch_bin), schema_no, version


def commit_of(path):
    try:
        p = subprocess.run(["git", "-C", path, "rev-parse", "HEAD"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                           text=True, timeout=30)
        return p.stdout.strip() if p.returncode == 0 and p.stdout.strip() else None
    except (OSError, subprocess.SubprocessError):
        return None
