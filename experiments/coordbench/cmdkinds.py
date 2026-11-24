"""Which tool calls are read-only coordination calls, whether a result had new content, board edits, and the resolution of a
klatch call made through a shell variable, alias or function (#1524, metric_version 2). Standard library only; nothing here
starts a process.

The idle-turn definition (SPEC section 5) lives on this file: a turn is idle when every tool call is a read-only coordination
call (read_kinds) whose result had no new content (is_new)."""
import json
import re

BOARD = "TASKS.md"
RESOLVE_LIMIT = ("a klatch call is recognised when the command string, after substituting a variable, alias or function defined "
                 "in that same command, contains the klatch binary path; a wrapper script, a variable set in an earlier command "
                 "(each Bash call is a fresh shell) or a path built at run time is a limit: those calls are not counted")


def _binre(klatch_bin):
    return re.escape(klatch_bin) if klatch_bin else r"klatch"


# -- resolving a wrapped klatch call --------------------------------------------------------------------------------------

ASSIGN = re.compile(r"""(?:^|[;&\s])(?:export\s+|local\s+|declare\s+)?([A-Za-z_]\w*)=("[^"]*"|'[^']*'|[^\s;&|]+)""")
ALIAS = re.compile(r"""\balias\s+([A-Za-z_][\w-]*)=("[^"]*"|'[^']*')""")
FUNCTION = re.compile(r"(?:\bfunction\s+)?([A-Za-z_][\w-]*)\s*\(\)\s*\{([^}]*)\}")


def _unquote(v):
    v = v.strip()
    return v[1:-1] if len(v) >= 2 and v[0] == v[-1] and v[0] in "\"'" else v


def _variables(cmd, b):
    return {m.group(1): _unquote(m.group(2)) for m in ASSIGN.finditer(cmd) if re.search(b, m.group(2))}


def _command_names(cmd, b):
    """{name: what it runs} for the aliases and functions defined in cmd whose text runs the binary."""
    names = {m.group(1): _unquote(m.group(2)) for m in ALIAS.finditer(cmd) if re.search(b, m.group(2))}
    for m in FUNCTION.finditer(cmd):
        call = re.search(r"\S*" + b + r"\S*", m.group(2))
        if call:
            names[m.group(1)] = _unquote(call.group(0))
    return names


def _replace_variable(cmd, name, val):
    return re.sub(r"\"\$\{%s\}\"|\"\$%s\"|\$\{%s\}|\$%s\b" % ((name,) * 4), lambda _: val, cmd)


def _replace_command(cmd, name, val):
    """The name where a command starts (line start or after ; & | or an opening parenthesis) becomes what it runs."""
    return re.sub(r"(^|[;&|(\n])(\s*)%s(?=\s)" % re.escape(name), lambda m: m.group(1) + m.group(2) + val, cmd, flags=re.M)


def resolve_command(cmd, klatch_bin):
    """cmd with a variable, alias or function that stands for the klatch binary replaced by its path, when it is defined in
    cmd itself (RESOLVE_LIMIT). Anything else comes back unchanged."""
    if not isinstance(cmd, str) or not cmd:
        return cmd
    b = _binre(klatch_bin)
    out = cmd
    for name, val in _variables(cmd, b).items():
        out = _replace_variable(out, name, val)
    for name, val in _command_names(cmd, b).items():
        out = _replace_command(out, name, val)
    return out


# -- board edits ------------------------------------------------------------------------------------------------------

FILE_WRITERS = ("Edit", "Write", "MultiEdit", "NotebookEdit")
BOARD_WRITE = re.compile(r">>?\s*['\"]?\S*TASKS\.md|\btee\b[^|;&\n]*TASKS\.md|\bsed\s+(?:-\S+\s+)*-i\S*[^|;&\n]*TASKS\.md"
                         r"|\b(?:mv|cp)\b[^|;&\n]*\sTASKS\.md\b|\bpython3?\b[^\n]*open\([^)]*TASKS\.md[^)]*['\"][wa]")


def _path_of(inp):
    return str(inp.get("file_path") or inp.get("notebook_path") or "").replace("\\", "/").rsplit("/", 1)[-1]


def is_board_edit(name, inp):
    """True when the tool call changes the task board: a file tool on TASKS.md or a shell command that writes it (B1's
    equivalent of a message: SPEC 5 counts board-line edits)."""
    if name in FILE_WRITERS:
        return _path_of(inp) == BOARD
    cmd = inp.get("command", "")
    return name == "Bash" and BOARD in str(cmd) and bool(BOARD_WRITE.search(cmd))


# -- read-only coordination calls and new content -------------------------------------------------------------------------

STREAM_KINDS = frozenset(("klatch:read", "klatch:wait", "klatch:digest", "git:pull", "git:fetch"))  # print only what is new
STATE_KINDS = frozenset(("klatch:claims", "klatch:asks", "klatch:who", "klatch:board", "klatch:history", "klatch:stuck",
                         "klatch:hooks", "git:log", "board", "listagents"))  # print the whole state every time
NO_NEWS = re.compile(r"\s*(?:klatch:\s*)?(?:no unread|nothing new|no new\b|timed out|already up[ -]to[ -]date|nothing to)", re.I)
PART_SPLIT = re.compile(r"&&|\|\||;|\||\n")
BOARD_READERS = re.compile(r"^(?:cat|head|tail|less|more|nl|grep|egrep|rg|sed\s+-n|awk|wc|bat)\b")
LINE_PREFIX = re.compile(r"(?m)^\s*\d+[→\t]")
NEUTRAL = re.compile(r"^(?:cd\b|alias\b|(?:export\s+)?[A-Za-z_]\w*=(?:\"[^\"]*\"|'[^']*'|\S*)$|(?:function\s+)?[\w-]+\s*\(\)\s*\{)")


def _klatch_kind(part, klatch_bin):
    m = re.match(r"^(?:\S+=\S+\s+)*" + _binre(klatch_bin)
                 + r"\s+(?:-\S+\s+)*(read|wait|digest|board|history|who|claims|asks|stuck|hooks)\b", part)
    return "klatch:" + m.group(1) if m else None


def _git_kind(part):
    m = re.match(r"^git\s+(?:-C\s+\S+\s+)?(pull|fetch|log)\b", part)
    return "git:" + m.group(1) if m else None


def _part_kind(part, klatch_bin):
    """The kind of one simple command: "" for one with no output and no coordination (a cd, an assignment, the head or tail of
    a function definition), a kind name for a read-only coordination call, None for anything else."""
    part = part.strip()
    if not part or part == "}" or NEUTRAL.match(part):
        return ""
    if re.match(r"^sleep\b", part):
        return "sleep"
    kind = _klatch_kind(part, klatch_bin) or _git_kind(part)
    if kind is None and BOARD in part and BOARD_READERS.match(part):
        kind = "board"
    return kind


def _bash_kinds(cmd, klatch_bin):
    if not isinstance(cmd, str):
        return None
    kinds = [_part_kind(p, klatch_bin) for p in PART_SPLIT.split(cmd)]
    return None if None in kinds else ([k for k in kinds if k] or None)


def read_kinds(name, inp, klatch_bin):
    """The read-only coordination kinds of one tool call, or None when the call is anything else. The per-arm equivalents of
    a read-only coordination turn: K: klatch read, claims, asks, who, wait, digest, board, history; B1: reading TASKS.md
    (the Read tool, cat), git pull to look at the board, ListAgents; every arm: git fetch, git log and sleep."""
    if name == "ListAgents":
        return ["listagents"]
    if is_board_edit(name, inp):
        return None
    if name in ("Read", "Grep", "Glob"):
        return ["board"] if BOARD in json.dumps(inp) else None
    return _bash_kinds(inp.get("command"), klatch_bin) if name == "Bash" else None


def normalize(text):
    return LINE_PREFIX.sub("", text or "").strip()


def is_new(text, kinds, previous):
    """Whether the result of a read-only coordination call held new content. Nothing, or a no-news phrase (klatch's empty read
    note, a wait that timed out, `Already up to date.`), is not new. A call that prints only what is new (klatch read, wait,
    git pull) has new content when it printed anything else. A call that prints the whole state (the board, ListAgents,
    claims, who) has new content when the text differs from the last result of the same call by this agent (the first
    result is new). `previous` is that last normalised text or None."""
    t = normalize(text)
    if not t:
        return False
    changed = previous is None or t != previous
    if STATE_KINDS.intersection(kinds):
        return changed
    if NO_NEWS.match(t):
        return False
    return True if STREAM_KINDS.intersection(kinds) else changed
