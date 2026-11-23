"""Render service settings as a small text block. All of the logic lives in this one file on purpose.

The option table OPTIONS says which options exist, their defaults and how each is shown. render() prints the header line
and then one `name = value` line per option, in alphabetical order of the names.
"""

# The option table: name -> (default, kind). One row per option. The kind says how format_value shows the value.
OPTIONS = {
    "host": ("localhost", "text"),
    "port": (8080, "int"),
}


def option_names():
    """The option names in render order: alphabetical."""
    return sorted(OPTIONS)


def default_config():
    """A new dict with every option at its default."""
    return {name: spec[0] for name, spec in OPTIONS.items()}


def merge_config(overrides=None):
    """The defaults updated with overrides (a dict). An unknown option name raises KeyError."""
    config = default_config()
    for name, value in (overrides or {}).items():
        if name not in OPTIONS:
            raise KeyError(name)
        config[name] = value
    return config


def format_value(kind, value):
    """Show one value as text, by kind. The kinds today: "text" (str(value)) and "int" (the value as a whole number).
    An unknown kind raises ValueError."""
    if kind == "text":
        return str(value)
    if kind == "int":
        return str(int(value))
    # new kinds go above this line, one block per kind
    raise ValueError("unknown kind: %s" % kind)


def render_header(count):
    """The first line of the output. count is the number of options in the table."""
    return "[settings]"


def render_line(name, value):
    """One output line: `name = value`, the value shown by the kind the table gives that option."""
    return "%s = %s" % (name, format_value(OPTIONS[name][1], value))


def render(overrides=None):
    """The whole block as one string with no trailing newline: the header, then a line per option."""
    config = merge_config(overrides)
    lines = [render_header(len(OPTIONS))]
    for name in option_names():
        lines.append(render_line(name, config[name]))
    return "\n".join(lines)


def parse_config(text):
    """Read `name = value` lines into an overrides dict of strings. Blank lines and lines starting with # are skipped.
    A line without `=` raises ValueError, and so does an unknown option name (the message names it)."""
    overrides = {}
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            raise ValueError("not a name = value line: %r" % line)
        name, _, value = line.partition("=")
        name = name.strip()
        if name not in OPTIONS:
            raise ValueError("unknown option: %s" % name)
        overrides[name] = value.strip()
    return overrides


def describe():
    """One line per option for a help text: `name (kind, default D)`, alphabetical."""
    return "\n".join("%s (%s, default %s)" % (n, OPTIONS[n][1], OPTIONS[n][0]) for n in option_names())


# ---- helpers below this line are added by tickets, one section each ----
