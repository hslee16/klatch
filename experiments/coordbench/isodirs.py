"""The directories of a run and what the harness may do to them (split out of isolation.py, #1519): hiding (mode 0300) and
restoring the directories it made, who owns a run (pid markers), the sweep of what a killed run left, which directories it
accepts as --logs-dir and --tmp-root, and the minimum Claude Code version. Standard library only."""
import os
import re
import secrets
import shutil
import stat
import tempfile

USER_ROOTS = ("/Users", "/home", "/root")
OWNER_FILE = ".owner"
LIVE_PREFIX = ".live."
MIN_CLAUDE = (2, 1, 268)  # deny rules reached through a symlinked directory (macOS /var) apply from this version (docs, permissions)


def real_home():
    return os.path.realpath(os.path.expanduser("~"))


def token():
    return secrets.token_hex(8)


class IsolationError(RuntimeError):
    pass


_HIDDEN = set()  # the directories this process has hidden and not restored yet (restore_all, for a signal handler)


def hide(path):
    """Mode 0300: the owner can still create and open what is inside by name, nobody (the owner's agents included) can list
    it. Undone by restore() before the harness lists or removes the directory. Only directories the harness made."""
    try:
        os.chmod(path, 0o300)
    except OSError as e:
        raise IsolationError("cannot hide %s from the agents (chmod: %s); the run is refused rather than run unprotected" % (path, e))
    _HIDDEN.add(path)


def restore(path):
    _HIDDEN.discard(path)
    try:
        os.chmod(path, 0o700)
    except OSError:
        pass


def restore_all():
    for p in list(_HIDDEN):
        restore(p)


def pid_alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except OSError:
        return True
    return True


def _read_pid(path):
    try:
        with open(path) as f:
            return int(f.read().split()[0])
    except (OSError, ValueError, IndexError):
        return None


def write_owner(directory):
    """Mark a directory of the harness with the pid that owns it, so a later start can tell a crash from a live run."""
    with open(os.path.join(directory, OWNER_FILE), "w") as f:
        f.write("%d\n" % os.getpid())


def _restore_tree(path):
    restore(path)
    try:
        names = os.listdir(path)
    except OSError:
        return
    for n in names:
        p = os.path.join(path, n)
        if os.path.isdir(p) and not os.path.islink(p):
            _restore_tree(p)


def sweep_stale(tmp_root):
    """Restore the modes of and remove the cb-* directories of runs whose process is gone (a SIGKILL or a crash left them with
    mode 0300). A directory without an owner file, or whose owner is alive, is left alone."""
    try:
        names = os.listdir(tmp_root)
    except OSError:
        return []
    removed = []
    for n in names:
        d = os.path.join(tmp_root, n)
        pid = _read_pid(os.path.join(d, OWNER_FILE)) if n.startswith("cb-") else None
        if pid is not None and not pid_alive(pid):
            _restore_tree(d)
            shutil.rmtree(d, ignore_errors=True)
            removed.append(d)
    return removed


def live_marker(logs_root, tok):
    path = os.path.join(logs_root, LIVE_PREFIX + tok)
    with open(path, "w") as f:
        f.write("%d\n" % os.getpid())
    return path


def _markers(logs_root):
    """[(marker path, pid or None)] of the live-run markers under a logs root."""
    try:
        names = [n for n in os.listdir(logs_root) if n.startswith(LIVE_PREFIX)]
    except OSError:
        return []
    return [(os.path.join(logs_root, n), _read_pid(os.path.join(logs_root, n))) for n in names]


def _remove(path):
    try:
        os.unlink(path)
    except OSError:
        pass


def other_live(logs_root, tok):
    """True when another process has a run in progress under this logs root; markers of dead processes are removed."""
    mine = os.path.join(logs_root, LIVE_PREFIX + tok)
    others = [(m, pid) for m, pid in _markers(logs_root) if m != mine]
    for m, pid in others:
        if not pid_alive(pid or 0):
            _remove(m)
    return any(pid_alive(pid or 0) for _, pid in others)


def sweep_logs(logs_root):
    """Remove the staging directories (random names, never published) and the markers of runs that died: the marker names no live
    process."""
    for m, pid in _markers(logs_root):
        if not pid_alive(pid or 0):
            stage = os.path.join(logs_root, os.path.basename(m)[len(LIVE_PREFIX):])
            if os.path.isdir(stage):
                _restore_tree(stage)
                shutil.rmtree(stage, ignore_errors=True)
            _remove(m)


def _under_user_root(p):
    """The user root (as written or resolved, or the real home) that holds this resolved path, or None."""
    candidates = list(USER_ROOTS) + [os.path.realpath(x) for x in USER_ROOTS] + [real_home()]
    return next((r for r in candidates if p == r or p.startswith(r.rstrip(os.sep) + os.sep)), None)


def _ownership_problem(p, path, what):
    """Why an existing directory is not safe to make directories in (not yours, writable by group or others), or None."""
    st = os.stat(p)
    sticky_open = bool(st.st_mode & stat.S_ISVTX) and bool(st.st_mode & 0o002)
    if sticky_open:
        return None
    if st.st_uid != os.getuid():
        return "%s %s is not owned by you" % (what, path)
    if st.st_mode & 0o022:
        return "%s %s is writable by group or others (mode %o): use a directory only you can write" % (what, path, stat.S_IMODE(st.st_mode))
    return None


def validate_dir(path, what, strict=False):
    """Why the harness refuses this directory as --logs-dir or --tmp-root (it makes directories inside it, and hides some of
    them), or None. Not the home directory, not another user's, not writable by group or others (a sticky world-writable
    directory such as /tmp is the usual temp root and is accepted). strict (a real run in sandbox mode): not under /Users, /home,
    /root or the real home either, because the file-tool deny rules cannot cover a user root that holds the run (a deny beats
    the allow of the agent's own clone), which would leave the user's other directories readable to the file tools."""
    p = os.path.realpath(path)
    root = _under_user_root(p) if strict else None
    tmp = os.path.realpath(tempfile.gettempdir())
    if p == real_home():
        return "%s %s is your home directory: use a directory of its own" % (what, path)
    if root:
        return ("%s %s lies under %s: in sandbox mode the file-tool deny rules cannot cover a user root that holds the run; use a "
                "directory outside the user roots, for example --logs-dir %s --tmp-root %s (or --isolation layout, "
                "which gives up the sandbox)" % (what, path, root, os.path.join(tmp, "coordbench-logs"), os.path.join(tmp, "coordbench-tmp")))
    return _ownership_problem(p, path, what) if os.path.exists(p) else None


def version_problem(version):
    """Why a Claude Code of this version cannot run the sandbox settings (docs: deny rules through a symlinked directory apply
    from 2.1.268, and macOS temp directories are under the symlink /var), or None."""
    need = ".".join(str(x) for x in MIN_CLAUDE)
    parts = re.match(r"(\d+)\.(\d+)\.(\d+)", version or "")
    if not parts:
        return "cannot tell which Claude Code version this is; the sandbox settings need %s or later (use --isolation layout to run without them)" % need
    if tuple(int(x) for x in parts.groups()) < MIN_CLAUDE:
        return "Claude Code %s is older than %s, the first version whose deny rules apply through symlinked directories; upgrade it or use --isolation layout" % (version, need)
    return None


def stage_binary(src, tmp_root):
    """A copy of the klatch binary in a directory of its own under tmp_root; (path of the copy, the directory). The agents
    are given the copy's path, so no prompt names a path inside the klatch repository (#1519)."""
    d = tempfile.mkdtemp(prefix="cb-bin-", dir=tmp_root)
    write_owner(d)
    dest = os.path.join(d, os.path.basename(src))
    shutil.copy2(src, dest)
    os.chmod(dest, 0o755)
    return dest, d
