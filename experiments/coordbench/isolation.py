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
import secrets
import shlex
import shutil
import tempfile

USER_ROOTS = ("/Users", "/home", "/root")
VAR_PREFIX = r"(?:~|\$\{?HOME\}?|\$\{?TMPDIR\}?)"
_TAIL = r"""(?:/[^\s'"`;,()<>|&\\*?]*)"""
PATH_RE = re.compile(r"(?<![\w./~$:-])(%s%s*|\.\.%s*|\.%s+|%s+)" % (VAR_PREFIX, _TAIL, _TAIL, _TAIL, _TAIL))
SEGMENT_RE = re.compile(r"&&|\|\||;|\n|\|")
HEREDOC_RE = re.compile(r"<<-?\s*(['\"]?)(\w+)\1")
PROCESS_RE = re.compile(r"""(?:^|[;&|(\n]|\$\()\s*(?:ps|pgrep|lsof|pstree|pidof)(?=\s|$)|['"](?:ps|pgrep|lsof|pstree|pidof)['"]|/proc/""")
PATH_FIELDS = {"Read": ("file_path",), "Edit": ("file_path",), "MultiEdit": ("file_path",), "Write": ("file_path",),
               "NotebookEdit": ("notebook_path",), "Glob": ("path", "pattern"), "Grep": ("path", "glob")}


def _real(path):
    return os.path.realpath(path)


class Policy:
    """What one agent may name. allow: directories (or files) it was given; guard: directories it must not touch. A path
    is a violation when the longest matching entry is a guard entry, so an allowed directory inside a guarded one (the
    agent's clone inside the run scratch directory) is fine and its siblings are not."""

    def __init__(self, agent, clone, home, tmp, allow=(), guard=()):
        self.agent, self.clone, self.home, self.tmp = agent, clone, home, tmp
        self.entries = [("allow", p) for p in (clone, home, tmp) + tuple(allow)] + [("guard", p) for p in guard]
        self.entries += [("guard", r) for r in USER_ROOTS]
        self.entries = [(k, p) for k, p in self.entries if p]

    def kind_of(self, path):
        """"allow", "guard" or None for an absolute path (the path as written and with symlinks resolved)."""
        hit = {}
        for p in {os.path.normpath(path), _real(path)}:
            best = None
            for kind, base in self.entries:
                for b in {os.path.normpath(base), _real(base)}:
                    if p == b or p.startswith(b.rstrip(os.sep) + os.sep):
                        if best is None or len(b) > best[0] or (len(b) == best[0] and kind == "guard"):
                            best = (len(b), kind)
            hit[p] = best[1] if best else None
        return "guard" if "guard" in hit.values() else ("allow" if "allow" in hit.values() else None)

    def resolve(self, token, cwd):
        t = token
        for var, val in (("${HOME}", self.home), ("$HOME", self.home), ("${TMPDIR}", self.tmp), ("$TMPDIR", self.tmp)):
            if t.startswith(var):
                t = val + t[len(var):]
        if t == "~" or t.startswith("~/"):
            t = self.home + t[1:]
        return os.path.normpath(t if os.path.isabs(t) else os.path.join(cwd, t))


def strip_heredoc_files(command):
    """The command without the bodies of heredocs that only write a file (`cat > f <<EOF`, `tee f <<EOF`): code a builder
    writes into its own clone may say `..` without reading anything. A heredoc fed to python3 or a shell is kept."""
    lines, out, end = command.split("\n"), [], None
    for ln in lines:
        if end is not None:
            if ln.strip() == end:
                end = None
            continue
        out.append(ln)
        m = HEREDOC_RE.search(ln)
        if m and re.search(r"\b(cat|tee)\b", ln.split("<<")[0]) and not re.search(r"\b(python3?|sh|bash)\b", ln.split("<<")[0]):
            end = m.group(2)
    return "\n".join(out)


def _bash_paths(command, policy, cwd):
    """[(token, resolved path, cwd after the command)] for every path-like token of a shell command; `cd` is followed so
    that a `..` inside a subdirectory is resolved there."""
    found = []
    for seg in SEGMENT_RE.split(strip_heredoc_files(command)):
        try:
            words = shlex.split(seg, posix=True)
        except ValueError:
            words = seg.split()
        if words and words[0] in ("cd", "pushd"):
            target = words[1] if len(words) > 1 else "~"
            cwd = policy.resolve(target, cwd)
            found.append((target, cwd))
            continue
        for m in PATH_RE.finditer(seg):
            found.append((m.group(1), policy.resolve(m.group(1), cwd)))
    return found, cwd


def tool_calls(lines):
    """[(name, input)] of every tool_use block in stream-json lines, in order."""
    calls = []
    for ln in lines:
        try:
            ev = json.loads(ln)
        except ValueError:
            continue
        content = ((ev.get("message") or {}).get("content")) if isinstance(ev, dict) else None
        for b in content if isinstance(content, list) else []:
            if isinstance(b, dict) and b.get("type") == "tool_use":
                calls.append((b.get("name"), b.get("input") if isinstance(b.get("input"), dict) else {}))
    return calls


def scan_lines(lines, policy, session=None):
    """(number of calls checked, [violation]) for the stream lines of one session. A violation is a dict: agent, session,
    call (1-based), tool, text (the command or path, cut), path (what it resolves to) and why."""
    cwd, violations, n = policy.clone, [], 0
    for i, (name, inp) in enumerate(tool_calls(lines), 1):
        n += 1
        hits = []
        if name == "Bash":
            cmd = str(inp.get("command", ""))
            paths, cwd = _bash_paths(cmd, policy, cwd)
            hits += [(tok, p, "path outside the agent's own directories") for tok, p in paths if policy.kind_of(p) != "allow"
                     and (policy.kind_of(p) == "guard" or _is_parent(tok))]
            if PROCESS_RE.search(cmd):
                hits.append((cmd, "", "process listing (other agents' paths are in their command lines and environments)"))
            text = cmd
        else:
            text = ""
            for field in PATH_FIELDS.get(name, ()):
                v = inp.get(field)
                for m in ([x.group(1) for x in PATH_RE.finditer(v)] + ([v] if field != "pattern" else [])) if isinstance(v, str) else []:
                    p = policy.resolve(m, cwd)
                    if policy.kind_of(p) == "guard":
                        hits.append((m, p, "path outside the agent's own directories"))
                text = text or str(v)
        seen = set()
        for tok, p, why in hits:
            if (tok, why) in seen:
                continue
            seen.add((tok, why))
            violations.append({"agent": policy.agent, "session": session or policy.agent, "call": i, "tool": name,
                               "text": (text if name == "Bash" else tok)[:160], "path": p, "why": why})
    return n, violations


def _is_parent(token):
    return token == ".." or token.startswith("../")


def describe(violations, limit=3):
    """One line for the record's notes."""
    first = "; ".join("%s call %d %s %r (%s)" % (v["session"], v["call"], v["tool"], v["text"][:80], v["why"])
                      for v in violations[:limit])
    more = "" if len(violations) <= limit else " and %d more" % (len(violations) - limit)
    return "isolation violation (SPEC 11): %d out-of-bounds tool call(s): %s%s" % (len(violations), first, more)


def _slash(path):
    return "//" + _real(path).lstrip("/")


def sandbox_settings(iso):
    """The settings keys that restrict one agent to its own directories (merged into settings_for by the driver). iso is the
    dict the harness builds (runisolation): mode "sandbox" or "layout", own (read and write), write (more directories the agent writes: origin, the run store), read (read only), others (the
    other agents' directories) and guard (directories no agent may touch). Mode "layout" adds nothing: only the file
    permissions and the scan then protect the run."""
    if not iso or iso.get("mode") != "sandbox":
        return {}
    guard, own, read = iso.get("guard", []), iso.get("own", []), iso.get("read", [])
    # the file tools cannot be denied the scratch root (a deny beats the allow of the agent's own clone inside it): they get
    # the explicit guard list and the other agents' directories; the unlistable root and the random names do the rest
    file_deny = [r for p in [g for g in guard if g != iso.get("root")] + list(iso.get("others", [])) for r in ("Read(%s)" % _slash(p), "Read(%s/**)" % _slash(p),
                                                                                  "Edit(%s)" % _slash(p), "Edit(%s/**)" % _slash(p))]
    return {"sandbox": {"enabled": True, "failIfUnavailable": True, "allowUnsandboxedCommands": False,
                        "filesystem": {"denyRead": [_slash(p) for p in guard],
                                       "allowRead": [_slash(p) for p in list(own) + list(iso.get("write", [])) + list(read)],
                                       "allowWrite": [_slash(p) for p in list(own) + list(iso.get("write", []))]},
                        "network": {"allowLocalBinding": True}},
            "permissions": {"deny": file_deny}}


# layout helpers ------------------------------------------------------------------------------------------------------

def token():
    return secrets.token_hex(8)


def hide(path):
    """Mode 0300: the owner can still create and open what is inside by name, nobody (the owner's agents included) can list
    it. Undone by restore() before the harness lists or removes the directory."""
    os.chmod(path, 0o300)


def restore(path):
    try:
        os.chmod(path, 0o700)
    except OSError:
        pass


def stage_binary(src, tmp_root):
    """A copy of the klatch binary in a directory of its own under tmp_root; (path of the copy, the directory). The agents
    are given the copy's path, so no prompt names a path inside the klatch repository (#1519)."""
    d = tempfile.mkdtemp(prefix="cb-bin-", dir=tmp_root)
    dest = os.path.join(d, os.path.basename(src))
    shutil.copy2(src, dest)
    os.chmod(dest, 0o755)
    return dest, d
