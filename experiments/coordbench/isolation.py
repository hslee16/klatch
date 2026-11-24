"""Isolation of the agents from each other and from the harness (#1519, SPEC 11, "Contamination of the fixture").

Three layers, from the one that always holds to the one that is only as good as the platform:

  layout     every agent has a directory of its own under the run scratch directory, named with a random token. The
             scratch directory and the log directory have the mode 0300 (write and search, no read) while the agents run,
             so an agent that is not told a path cannot list its way to another agent's clone, the logs or the checkers.
             This is plain file permission, it covers every tool (Read, Glob, Grep, Bash, python3) and every OS.
  sandbox    the --settings JSON of a real agent (sandbox_settings below): Claude Code's OS-level sandbox for Bash and its
             child processes (Seatbelt on macOS, bubblewrap on Linux) with a read deny on the scratch root, the logs and
             the klatch repository, re-allowing only the agent's own directories; and Read/Edit deny rules for the file
             tools. Written from the documentation, never run with a model in the builder's session (see README, "Isolation").
  scan       the post-run scan (scan_lines below): every tool call in the streams is checked for paths outside what the
             agent was given. A hit makes the run invalid with a reason. This is detective, not preventive: a path built at
             run time inside a python3 program is not seen by it.

Standard library only. Nothing here reads a credential or starts a process.
"""
import json
import os
import re
import shlex
import shutil

from isodirs import (IsolationError, LIVE_PREFIX, MIN_CLAUDE, OWNER_FILE, USER_ROOTS, hide, live_marker, other_live,  # noqa: F401
                     pid_alive, real_home, restore, restore_all, stage_binary, sweep_logs, sweep_stale, token,
                     validate_dir, version_problem, write_owner)

VAR_PREFIX = r"(?:~|\$\{?HOME\}?|\$\{?TMPDIR\}?)"
_TAIL = r"""(?:/[^\s'"`;,()<>|&\\*?]*)"""
PATH_RE = re.compile(r"(?<![\w./~$:-])(%s%s*|\.\.%s*|\.%s+|%s+)" % (VAR_PREFIX, _TAIL, _TAIL, _TAIL, _TAIL))
SEGMENT_RE = re.compile(r"&&|\|\||;|\n|\|")
HEREDOC_RE = re.compile(r"<<-?\s*(['\"]?)(\w+)\1")
_PS = r"(?:ps|pgrep|lsof|pstree|pidof)"
PROCESS_RE = re.compile(r"""(?:^|[;&|(\n]|\$\()\s*%s(?=\s|$)|[\[(]\s*['"]%s['"]|(?:system|popen|run|call|check_output|Popen)\(\s*['"]%s\b|/proc/""" % (_PS, _PS, _PS))
# a bare / is a search root only as the argument of a search or listing command or call (not `3 / 4`)
ROOT_ARG_RE = re.compile(r"""(?:\b(?:find|rg|grep|ag|ack|ls|du|fd|tree|locate|cd|cat)\b[^;|&\n]*?\s|\b(?:listdir|walk|scandir|glob|iglob|rglob|Path|chdir)\(\s*['"])/(?=['"\s)]|$)""")
# a wildcard in the first segment (/U*/..., /*/...) or in the second one under a user or system root: a search from the root
ROOT_WILD_RE = re.compile(r"""(?<![\w./~$:*?\]-])(?:/[^/\s'"`;|&]*[*?\[]|/(?:Users|home|root|private|var|Volumes|U[^/\s]*|h[^/\s]*)/[^/\s'"`;|&]*[*?\[])""")
PATH_FIELDS = {"Read": ("file_path",), "Edit": ("file_path",), "MultiEdit": ("file_path",), "Write": ("file_path",),
               "NotebookEdit": ("notebook_path",), "Glob": ("path", "pattern"), "Grep": ("path", "glob")}
OUT_OF_BOUNDS = "path outside the agent's own directories"
ROOT_SEARCH = "search rooted at the filesystem root, a user root or a parent of the run"
PROCESS_LISTING = "process listing (other agents' paths are in their command lines and environments)"


def _real(path):
    return os.path.realpath(path)


def _variants(path):
    return {os.path.normpath(path), _real(path)}


def _contains(base, path):
    return path == base or path.startswith(base.rstrip(os.sep) + os.sep)


def real_home():
    return os.path.realpath(os.path.expanduser("~"))


def user_roots():
    """The roots that hold user files, with the harness user's real home (absolute, never ~: ~ is the agent's own HOME)."""
    return tuple(dict.fromkeys(USER_ROOTS + tuple(os.path.realpath(r) for r in USER_ROOTS) + (real_home(),)))


def _ancestors(path):
    out, p = set(), os.path.dirname(path)
    while p and p not in out:
        out.add(p)
        p = os.path.dirname(p)
    return out


def _bases(kind, paths):
    return [(kind, b) for p in paths if p for b in _variants(p)]


class Exact(str):
    """A directory an agent may name (list) but whose contents it may not: its own directory above its clone."""


class Policy:
    """What one agent may name. allow: directories (or files) it was given; guard: directories it must not touch. A path is a violation when the longest matching entry is a guard entry, so an allowed directory inside a guarded one
    (the agent's clone inside the run scratch directory) is fine and its siblings are not. A parent of a guarded directory (the
    filesystem root, the temp root) is a violation too: naming it is how a search from the root starts."""

    def __init__(self, agent, own, allow=(), guard=()):
        """own is (clone, home, tmp): the agent's working directory, its HOME and its TMPDIR. An allow entry that is an Exact
        may be named itself but not its contents."""
        self.agent, (self.clone, self.home, self.tmp) = agent, own
        guards = tuple(guard) + user_roots()
        allowed = [p for p in tuple(own) + tuple(allow) if p]
        plain = [p for p in allowed if not isinstance(p, Exact)]
        self.bases = _bases("allow", plain) + _bases("guard", guards)
        self.exact = {b for _, b in _bases("allow", [p for p in allowed if isinstance(p, Exact)])}
        self.ancestors = {a for _, b in _bases("guard", guards) for a in _ancestors(b)}

    def _kind_of_one(self, path):
        if path in self.exact:
            return "allow"
        hits = [(len(base), kind == "guard", kind) for kind, base in self.bases if _contains(base, path)]
        if hits:
            return max(hits)[2]  # the longest entry wins, a guard entry wins a tie
        return "guard" if path in self.ancestors else None

    def kind_of(self, path):
        """"allow", "guard" or None for an absolute path (the path as written and with symlinks resolved)."""
        kinds = {self._kind_of_one(p) for p in _variants(path)}
        return "guard" if "guard" in kinds else ("allow" if "allow" in kinds else None)

    def resolve(self, token, cwd):
        t = token
        for var, val in (("${HOME}", self.home), ("$HOME", self.home), ("${TMPDIR}", self.tmp), ("$TMPDIR", self.tmp)):
            if t.startswith(var):
                t = val + t[len(var):]
        if t == "~" or t.startswith("~/"):
            t = self.home + t[1:]
        return os.path.normpath(t if os.path.isabs(t) else os.path.join(cwd, t))


def _writes_only_a_file(line):
    before = line.split("<<")[0]
    return bool(re.search(r"\b(cat|tee)\b", before)) and not re.search(r"\b(python3?|sh|bash)\b", before)


def strip_heredoc_files(command):
    """The command without the bodies of heredocs that only write a file (`cat > f <<EOF`, `tee f <<EOF`): code a builder
    writes into its own clone may say `..` without reading anything. A heredoc fed to python3 or a shell is kept."""
    out, end = [], None
    for ln in command.split("\n"):
        if end is not None:
            end = None if ln.strip() == end else end
            continue
        out.append(ln)
        m = HEREDOC_RE.search(ln)
        if m and _writes_only_a_file(ln):
            end = m.group(2)
    return "\n".join(out)


def _segment_paths(seg, policy, cwd):
    """([(token, resolved path)], cwd after the segment): `cd` moves the working directory, anything else is scanned."""
    try:
        words = shlex.split(seg, posix=True)
    except ValueError:
        words = seg.split()
    if words and words[0] in ("cd", "pushd"):
        target = words[1] if len(words) > 1 else "~"
        cwd = policy.resolve(target, cwd)
        return [(target, cwd)], cwd
    bare_ok = bool(ROOT_ARG_RE.search(seg))
    return [(m.group(1), policy.resolve(m.group(1), cwd)) for m in PATH_RE.finditer(seg) if m.group(1) != "/" or bare_ok], cwd


def _bash_paths(command, policy, cwd):
    """[(token, resolved path)] for every path-like token of a shell command, and the cwd after it; `cd` is followed so
    that a `..` inside a subdirectory is resolved there."""
    found = []
    for seg in SEGMENT_RE.split(strip_heredoc_files(command)):
        paths, cwd = _segment_paths(seg, policy, cwd)
        found += paths
    return found, cwd


def _content_blocks(line):
    try:
        ev = json.loads(line)
    except ValueError:
        return []
    content = ((ev.get("message") or {}).get("content")) if isinstance(ev, dict) else None
    return content if isinstance(content, list) else []


def tool_calls(lines):
    """[(name, input)] of every tool_use block in stream-json lines, in order."""
    return [(b.get("name"), b.get("input") if isinstance(b.get("input"), dict) else {})
            for ln in lines for b in _content_blocks(ln) if isinstance(b, dict) and b.get("type") == "tool_use"]


def _is_parent(token):
    return token == ".." or token.startswith("../")


def _bash_hits(command, policy, cwd):
    """([(token, path, why)], cwd after the command) for one Bash call."""
    paths, cwd = _bash_paths(command, policy, cwd)
    hits = [(tok, p, OUT_OF_BOUNDS if p not in policy.ancestors else ROOT_SEARCH) for tok, p in paths
            if policy.kind_of(p) == "guard" or (policy.kind_of(p) is None and _is_parent(tok))]
    if PROCESS_RE.search(command):
        hits.append((command, "", PROCESS_LISTING))
    hits += [(m.group(0), "", ROOT_SEARCH) for m in ROOT_WILD_RE.finditer(command)]
    return hits, cwd


def _field_tokens(field, value):
    """The paths a file-tool field names: path-like words in it and, for a path field, the whole value."""
    found = [m.group(1) for m in PATH_RE.finditer(value) if m.group(1) != "/"]  # the / of **/*.py is not a root
    return found + ([value] if field != "pattern" or value == "/" else [])


def _file_hits(name, inp, policy, cwd):
    hits = []
    for field in PATH_FIELDS.get(name, ()):
        value = inp.get(field)
        for tok in _field_tokens(field, value) if isinstance(value, str) else []:
            p = policy.resolve(tok, cwd)
            if policy.kind_of(p) == "guard":
                hits.append((tok, p, OUT_OF_BOUNDS if p not in policy.ancestors else ROOT_SEARCH))
        if isinstance(value, str) and ROOT_WILD_RE.search(value):
            hits.append((value, "", ROOT_SEARCH))
    return hits


def scan_lines(lines, policy, session=None):
    """(number of calls checked, [violation]) for the stream lines of one session. A violation is a dict: agent, session,
    call (1-based), tool, text (the command or path, cut), path (what it resolves to) and why."""
    cwd, violations, n = policy.clone, [], 0
    for n, (name, inp) in enumerate(tool_calls(lines), 1):
        if name == "Bash":
            command = str(inp.get("command", ""))
            hits, cwd = _bash_hits(command, policy, cwd)
        else:
            command, hits = "", _file_hits(name, inp, policy, cwd)
        for tok, p, why in _distinct(hits):
            violations.append({"agent": policy.agent, "session": session or policy.agent, "call": n, "tool": name,
                               "text": (command or tok)[:160], "path": p, "why": why})
    return n, violations


def _distinct(hits):
    seen, out = set(), []
    for tok, p, why in hits:
        if (tok, why) not in seen:
            seen.add((tok, why))
            out.append((tok, p, why))
    return out


def describe(violations, limit=3):
    """One line for the record's notes."""
    first = "; ".join("%s call %d %s %r (%s)" % (v["session"], v["call"], v["tool"], v["text"][:80], v["why"])
                      for v in violations[:limit])
    more = "" if len(violations) <= limit else " and %d more" % (len(violations) - limit)
    return "isolation violation (SPEC 11): %d out-of-bounds tool call(s): %s%s" % (len(violations), first, more)


def _slash(path):
    """The absolute-path form of the settings (//path). Directories are given with their symlinks resolved; the user roots as
    written and resolved (/home is a symlink on macOS)."""
    return "//" + (path if path in user_roots() else _real(path)).lstrip("/")


def _holds(root, dirs):
    return any(_contains(_real(root), _real(d)) for d in dirs)


def _file_deny(iso):
    """Read and Edit deny rules for the file tools. They cannot be denied a directory that holds the agent's own (a deny beats
    the allow of the clone inside it): not the scratch root, and not a user root that contains the run. They get the explicit
    guard list, the other agents' directories and the user roots with absolute paths; the unlistable agents directory and
    the random names do the rest."""
    mine = list(iso.get("own", [])) + list(iso.get("write", [])) + list(iso.get("read", []))
    roots = [r for r in user_roots() if not _holds(r, mine)]
    dirs = [g for g in iso.get("guard", []) if g != iso.get("root")] + list(iso.get("others", [])) + roots
    return ["%s(%s%s)" % (tool, _slash(p), suffix) for p in dirs for tool in ("Read", "Edit") for suffix in ("", "/**")]


def _filesystem(iso):
    own, write = list(iso.get("own", [])), list(iso.get("write", []))
    return {"denyRead": [_slash(p) for p in list(iso.get("guard", [])) + list(user_roots())],
            "allowRead": [_slash(p) for p in own + write + list(iso.get("read", []))],
            "allowWrite": [_slash(p) for p in own + write]}


def sandbox_settings(iso):
    """The settings keys that restrict one agent to its own directories (merged into settings_for by the driver). iso is the
    dict the harness builds (runisolation): mode "sandbox" or "layout", own (read and write), write (more directories the agent
    writes: origin, the run store, a role member's restart directory), read (read only: the staged binary, the python and git
    installations), others (the other agents' directories) and guard (directories no agent may touch). The real home, /Users,
    /home and /root are denied for reading and only the run's own directories are re-opened. No network key: the klatch
    binary opens no socket, and allowLocalBinding would open every localhost port on macOS. Mode "layout" adds nothing."""
    if not iso or iso.get("mode") != "sandbox":
        return {}
    return {"sandbox": {"enabled": True, "failIfUnavailable": True, "allowUnsandboxedCommands": False,
                        "filesystem": _filesystem(iso)},
            "permissions": {"deny": _file_deny(iso)}}


def tool_roots():
    """The installation directories of python3 and git (the agents run them, they may live under a user root)."""
    out = []
    for cmd in ("python3", "git"):
        exe = shutil.which(cmd)
        if exe:
            d = os.path.dirname(os.path.realpath(exe))
            out.append(os.path.dirname(d) if os.path.basename(d) in ("bin", "sbin") else d)
    return list(dict.fromkeys(r for r in out if os.path.isdir(r)))


