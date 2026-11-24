"""What the fixture hash covers (#1516, #1510) and what the report does with the hash version of a record.

FIXTURE_HASH_VERSION 2 (records of version 1 have no `fixture_hash_version` field):

  covered     every file of the fixture directory an agent can receive or that decides the outcome: setup.sh, template/, the
              task texts, private/, seed and history files, fixture.json (with the declared `generator_sources`), and
              reference/ (agents never get it, but the checker is validated against it and the fake agents replay it, so a
              change of it is a change of the oracle); plus, for a generated fixture (scale) the generator's OUTPUT (the
              template setup.sh writes, the reference, the tasks and the scenario map), and for a prebuilt history the commit
              id the setup makes in each arm (B0 and B1)
  not covered generator sources (fixture.json `generator_sources`: gen.py, gen_facts.py and the modules they import,
              build_history.py) because their output is committed and `--check` proves it, or is hashed as above; so
              refactoring a generator changes no hash unless it changes what it makes; `__pycache__`, `*.pyc`, `*.pyo` and
              editor leftovers (`.DS_Store`, `*~`, `*.swp`, `#*#`, `.#*`)
  checker     the hidden checker directory checks/<name>/ is hashed apart as `fixture.checker_sha256` (same skip rules)
  prompts     `prompt_sha256` {agent: sha256 of the agent's built prompt}, with the run's own working directory, binary path,
              room and restart command left as placeholders, so it says which arm text and task an agent was given

Version 1 hashed the whole directory, generator sources and bytecode included, and not the checker. The report never pools
records of different versions (split_scenarios), exactly like metric_version (metricver.py)."""
import hashlib
import os

import metricver

FIXTURE_HASH_VERSION = 2

SKIP_DIRS = ("__pycache__",)
SKIP_SUFFIXES = (".pyc", ".pyo", "~", ".swp", ".swo", ".swx")
SKIP_NAMES = (".DS_Store",)


def skipped(name):
    """True for a file that is never hashed: bytecode and editor leftovers."""
    return name in SKIP_NAMES or name.endswith(SKIP_SUFFIXES) or (name.startswith("#") and name.endswith("#")) \
        or name.startswith(".#")


def tree_update(h, root, omit=()):
    """Feed relative paths and bytes of the files under root (sorted; skipped files and the paths in omit left out) into h."""
    for d, dirs, files in os.walk(root):
        dirs[:] = sorted(x for x in dirs if x not in SKIP_DIRS)
        for n in sorted(files):
            p = os.path.join(d, n)
            rel = os.path.relpath(p, root)
            if skipped(n) or rel in omit:
                continue
            h.update(rel.encode() + b"\0")
            with open(p, "rb") as f:
                h.update(f.read() + b"\0")


def tree_sha256(root, omit=()):
    h = hashlib.sha256()
    tree_update(h, root, omit)
    return h.hexdigest()


def version_of(rec):
    v = rec.get("fixture_hash_version")
    return v if isinstance(v, int) and not isinstance(v, bool) else 1


def condition(records):
    """{"fixture hash version": [...], "fixture checker sha256": [...]} for the fields any record carries, in order of
    appearance (records made before version 2 have neither, so a report of only those is unchanged)."""
    out = {}
    for label, get in (("fixture hash version", lambda r: r.get("fixture_hash_version")),
                       ("fixture checker sha256", lambda r: (r.get("fixture") or {}).get("checker_sha256"))):
        seen = []
        for r in records:
            v = get(r)
            if v is not None and v not in seen:
                seen.append(v)
        if seen:
            out[label] = seen
    return out


def split_scenarios(records):
    """(records for the per-scenario analysis, counts per version or None): an older record's scenario gets the suffix
    ` [fixture hash vN]` when the versions are mixed (metricver.split_by)."""
    return metricver.split_by(records, version_of, "fixture_hash_version", " [fixture hash v%d]")


def newest_only(records):
    return metricver.newest_by(records, version_of)


def header_lines(counts):
    """The header lines of a report whose records have mixed fixture hash versions, else []."""
    if not counts or len(counts) < 2:
        return []
    parts = ", ".join("version %s: %d runs" % (v, counts[v]) for v in sorted(counts))
    return ["**Mixed fixture hash versions** (%s). Records of different fixture hash versions are not pooled: the older ones "
            "are listed as `SCENARIO [fixture hash vN]` and the scaling table uses the newest only. See fixhash.py for what "
            "the hash covers." % parts, ""]


def mismatch(new, old):
    """A one-line note when two analyses were made from different fixture hash versions, or None."""
    a = sorted((new.get("fixture_hash_versions") or {"1": 1}).keys())
    b = sorted((old.get("fixture_hash_versions") or {"1": 1}).keys())
    return None if a == b else ("the analyses use different fixture hash versions (now %s, before %s): the fixture hashes "
                                "are not comparable" % (",".join(a), ",".join(b)))


def version_notes(new, old):
    """The notes compare() adds for two analyses made from different versions: {"metric_version_mismatch": ...,
    "fixture_hash_version_mismatch": ...}, only for those that apply."""
    notes = {"metric_version_mismatch": metricver.mismatch(new, old), "fixture_hash_version_mismatch": mismatch(new, old)}
    return {k: v for k, v in notes.items() if v}


def comparison_lines(cmp_):
    """The markdown lines that state those notes in a comparison."""
    lines = []
    for key, label in (("metric_version_mismatch", "Different metric versions"),
                       ("fixture_hash_version_mismatch", "Different fixture hash versions")):
        if cmp_.get(key):
            lines += ["**%s**: %s." % (label, cmp_[key]), ""]
    return lines


def mixed_version_lines(a):
    """The header lines of a report whose records have mixed metric versions or mixed fixture hash versions."""
    return metricver.header_lines(a.get("metric_versions")) + header_lines(a.get("fixture_hash_versions"))
