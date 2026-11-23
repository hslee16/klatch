#!/usr/bin/env python3
"""Parity check for the coordbench arm prompts (SPEC section 2, rules 2, 3 and 5; ticket #1365).

Checks, with the standard library only and no model:
  1. length: every arm's coordination section is within TOLERANCE (default 15 percent) of the mean length, in words, of
     the base arms B0, B1, K1, K2; and the role arms B1-roles and K3 match section by section (worker, integrator,
     supervisor) within the same tolerance of their pair's mean;
  2. outline: each base arm has the six topic labels, in the same order;
  3. the common prompt: one coordination slot, and nothing arm-specific in it;
  4. the composed prompts: with the same inputs, the text outside the coordination section is identical in every arm;
  5. message prefixes: B1 and B1-roles use the shared ones, B0 none;
  6. no klatch words in a B arm, and no arm names another arm's tools; placeholders limited to the ones the harness fills.

The same checks run a second time for the T-shared variants (#1427): common-shared.md and B0-shared.md, B1-shared.md,
K1-shared.md, K2-shared.md (agents in one working directory; the role arms have no shared variant). The shared files must
also not say "own clone" or tell an agent to push, and must say that colleagues edit the same files in the same directory.

Usage: python3 check_parity.py [--arms-dir DIR] [--tolerance 0.15]    (exit 0 when every check passes, 1 otherwise)
"""
import argparse
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
BENCH = os.path.dirname(HERE)

BASE_ARMS = ("B0", "B1", "K1", "K2")
ROLE_ARMS = ("B1-roles", "K3")
ROLE_SECTIONS = ("worker", "integrator", "supervisor")
OUTLINE = ("Telling others what you are doing:", "Not stepping on each other's work:", "Asking and answering questions:",
           "Handing work over:", "Waiting:", "Finishing:")
SLOT = "{{COORDINATION_SECTION}}"
SHARED = "-shared"
COMMON_PLACEHOLDERS = {"TEST_COMMAND", "RESTART_COMMAND"}
KLATCH_PLACEHOLDERS = {"KLATCH", "ROOM"}
# the message prefixes the arms with messaging share (#1365 decision of 2026-11-23): the words klatch's convention and the K
# arms use. They are plain words, not klatch words. B0 has no messaging tool and uses none; the role arms add the K3 verbs.
SHARED_PREFIXES = ("seeking:", "answer:", "handoff:")
ROLE_PREFIXES = ("ready:", "merged:", "dropped:")
K_FORMS = ("seeking:", "answer", "handoff:")
# words that must not appear in a B arm (case-insensitive); the B arms are the control for klatch
KLATCH_WORDS = (r"klatch", r"\broom\b", r"\blobby\b", r"KLATCH_[A-Z_]+")
# tools that belong to one arm only: (pattern, arms that may name it)
ARM_TOOLS = (
    (r"SendMessage|ListAgents", {"B1", "B1-roles"}),
    (r"TASKS\.md", {"B1", "B1-roles"}),
)


def strip_comments(text):
    return re.sub(r"<!--.*?-->", "", text, flags=re.S).strip()


def words(text):
    return len(strip_comments(text).split())


def read(arms_dir, name):
    with open(os.path.join(arms_dir, name + ".md")) as f:
        return f.read()


def split_role_sections(text):
    """The '## ' sections of a role file, keyed worker, integrator or supervisor by the first word of the heading."""
    out = {}
    for part in re.split(r"^## ", strip_comments(text), flags=re.M)[1:]:
        heading, _, body = part.partition("\n")
        key = heading.strip().split()[0].lower()
        out[key] = body.strip()
    return out


def within(counts, tolerance):
    """(mean, {name: (count, deviation, ok)}) for lengths against their mean."""
    mean = sum(counts.values()) / float(len(counts))
    return mean, {k: (v, (v - mean) / mean, abs(v - mean) <= tolerance * mean) for k, v in counts.items()}


def check_lengths(arms_dir, tolerance, sfx=""):
    problems, lines = [], []
    texts = {a: read(arms_dir, a + sfx) for a in BASE_ARMS}
    mean, res = within({a: words(t) for a, t in texts.items()}, tolerance)
    lines.append("base arms%s: mean %.1f words, tolerance %d percent" % (sfx, mean, round(tolerance * 100)))
    for a in BASE_ARMS:
        n, dev, ok = res[a]
        lines.append("  %-9s %4d words  %+5.1f%%  %s" % (a, n, dev * 100, "ok" if ok else "OUT OF TOLERANCE"))
        if not ok:
            problems.append("%s is %d words, %+.1f%% from the mean %.1f" % (a, n, dev * 100, mean))
    secs = {a: split_role_sections(read(arms_dir, a)) for a in ROLE_ARMS} if not sfx else {}
    for key in ROLE_SECTIONS if not sfx else ():
        missing = [a for a in ROLE_ARMS if key not in secs[a]]
        if missing:
            problems.append("role section '%s' missing in %s" % (key, ", ".join(missing)))
            continue
        mean, res = within({a: len(secs[a][key].split()) for a in ROLE_ARMS}, tolerance)
        for a in ROLE_ARMS:
            n, dev, ok = res[a]
            lines.append("  roles %-10s %-9s %4d words  %+5.1f%%  %s" % (key, a, n, dev * 100, "ok" if ok else "OUT OF TOLERANCE"))
            if not ok:
                problems.append("%s %s section is %d words, %+.1f%% from the pair mean %.1f" % (a, key, n, dev * 100, mean))
    return problems, lines


def check_outline(arms_dir, sfx=""):
    problems = []
    for a in BASE_ARMS:
        text = strip_comments(read(arms_dir, a + sfx))
        pos = [text.find(label) for label in OUTLINE]
        for label, p in zip(OUTLINE, pos):
            if p < 0:
                problems.append("%s lacks the topic '%s'" % (a + sfx, label))
        found = [p for p in pos if p >= 0]
        if found != sorted(found):
            problems.append("%s has the topics in a different order" % (a + sfx))
    return problems


def check_prefixes(arms_dir, sfx=""):
    """B1 and B1-roles use the shared message prefixes (and the role arms the role verbs); B0 uses none; K1 and K2 say the
    same words, so that B1 is not given a vocabulary klatch does not use."""
    problems = []
    for a, want in (("B1", SHARED_PREFIXES), ("B1-roles", SHARED_PREFIXES + ROLE_PREFIXES)):
        if sfx and a != "B1":
            continue
        text = strip_comments(read(arms_dir, a + sfx))
        for prefix in want:
            if prefix not in text:
                problems.append("%s lacks the message prefix '%s'" % (a + sfx, prefix))
    text = strip_comments(read(arms_dir, "B0" + sfx))
    for prefix in SHARED_PREFIXES:
        if prefix in text:
            problems.append("B0%s has no messaging tool but uses the message prefix '%s'" % (sfx, prefix))
    for a in ("K1", "K2"):
        text = strip_comments(read(arms_dir, a + sfx))
        for form in K_FORMS:
            if form not in text:
                problems.append("%s no longer says '%s', so the B1 prefixes differ from klatch's" % (a + sfx, form))
    return problems


def check_common(arms_dir, sfx=""):
    text = strip_comments(read(arms_dir, "common" + sfx))
    problems = []
    if text.count(SLOT) != 1:
        problems.append("common%s.md must contain %s exactly once (found %d)" % (sfx, SLOT, text.count(SLOT)))
    for pat, _ in ARM_TOOLS:
        if re.search(pat, text):
            problems.append("common%s.md names an arm-specific tool (%s)" % (sfx, pat))
    if re.search("klatch", text, flags=re.I):
        problems.append("common%s.md names klatch" % sfx)
    return problems


def check_words_and_placeholders(arms_dir, sfx=""):
    problems = []
    for a in BASE_ARMS + (() if sfx else ROLE_ARMS):
        text = strip_comments(read(arms_dir, a + sfx))
        a = a + sfx
        found = set(re.findall(r"\{\{([A-Z_]+)\}\}", text))
        allowed = set(COMMON_PLACEHOLDERS)
        if a.startswith("K"):
            allowed |= KLATCH_PLACEHOLDERS
        for ph in sorted(found - allowed):
            problems.append("%s uses the placeholder {{%s}}, which this arm may not use" % (a, ph))
        if a.startswith("B"):
            for pat in KLATCH_WORDS:
                m = re.search(pat, text, flags=re.I)
                if m:
                    problems.append("B arm %s contains the klatch word '%s'" % (a, m.group(0)))
        for pat, owners in ARM_TOOLS:
            if a[: len(a) - len(sfx)] not in owners and re.search(pat, text):
                problems.append("%s names a tool of another arm (%s)" % (a, pat))
    return problems


def check_composition(arms_dir, sfx=""):
    """The prompts built by run.build_prompt are identical outside the coordination section."""
    if BENCH not in sys.path:
        sys.path.insert(0, BENCH)
    import run
    problems = []
    heads, tails = {}, {}
    for a in BASE_ARMS:
        kw = dict(arms_dir=arms_dir, arm=a, name="a", team=["a", "b"], task="TASK", workdir="/w", test_command="T",
                  klatch_bin="/k/klatch", room="r", max_turns=30, max_minutes=20,
                  topology="T-shared" if sfx else "T-clone")
        try:
            full = run.build_prompt(**kw)
        except ValueError as e:
            problems.append("the %s prompt cannot be built: %s" % (a + sfx, e))
            continue
        section = re.sub(r"<!--.*?-->", "", read(arms_dir, a + sfx), flags=re.S).strip()
        section = section.replace("{{KLATCH}}", "/k/klatch").replace("{{ROOM}}", "r")
        section = section.replace("{{TEST_COMMAND}}", "T").replace("{{RESTART_COMMAND}}", "")
        if section not in full:
            problems.append("the %s section does not appear verbatim in its built prompt" % (a + sfx))
            continue
        heads[a], _, tails[a] = full.partition(section)
    for a in BASE_ARMS:
        if a in heads and (heads[a] != heads[BASE_ARMS[0]] or tails[a] != tails[BASE_ARMS[0]]):
            problems.append("the text outside the coordination section differs between %s and %s" % (a + sfx, BASE_ARMS[0] + sfx))
    return problems


def check_shared_wording(arms_dir):
    """The T-shared files describe one working directory: no own clone, no push, and the common part says that colleagues
    edit the same files in the same directory right now (#1427)."""
    problems = []
    for name in ("common",) + BASE_ARMS:
        text = strip_comments(read(arms_dir, name + SHARED))
        for pat in (r"own clone", r"git push", r"\bclone\b", r"origin"):
            if re.search(pat, text, flags=re.I):
                problems.append("%s%s.md says '%s', which is wrong in one shared working directory" % (name, SHARED, pat.replace("\\b", "")))
    common = strip_comments(read(arms_dir, "common" + SHARED))
    for needle in ("same directory", "editing the same files"):
        if needle not in common:
            problems.append("common%s.md does not say '%s'" % (SHARED, needle))
    return problems


def run_all(arms_dir, tolerance, sfx=""):
    problems, report = [], []
    p, lines = check_lengths(arms_dir, tolerance, sfx)
    problems += p
    report += lines
    problems += check_outline(arms_dir, sfx)
    problems += check_prefixes(arms_dir, sfx)
    problems += check_common(arms_dir, sfx)
    problems += check_words_and_placeholders(arms_dir, sfx)
    problems += check_composition(arms_dir, sfx)
    if sfx == SHARED:
        problems += check_shared_wording(arms_dir)
    return problems, report


def run_everything(arms_dir, tolerance):
    """The clone prompts and, when the shared files exist, the T-shared prompts: (problems, report lines)."""
    problems, report = run_all(arms_dir, tolerance)
    p, r = run_all(arms_dir, tolerance, SHARED)
    return problems + p, report + ["", "T-shared variants:"] + r


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--arms-dir", default=HERE)
    ap.add_argument("--tolerance", type=float, default=0.15)
    a = ap.parse_args(argv)
    problems, report = run_everything(a.arms_dir, a.tolerance)
    print("\n".join(report))
    if problems:
        print("\nFAIL")
        for p in problems:
            print("  - " + p)
        return 1
    print("\nOK: lengths, outline, common prompt, composition, klatch words and placeholders all pass, for the clone and the shared prompts")
    return 0


if __name__ == "__main__":
    sys.exit(main())
