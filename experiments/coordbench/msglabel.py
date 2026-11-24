"""Keep an unverified B1 result from being read as a clean one (#1520 review). A B1 or B1-roles run whose record has
messaging_verified.status other than "verified" (unknown, or no such field in a record made before the check) proves nothing about
native messaging: it may be B0 plus a board. The report labels the arm of any group that holds one, and says at the top that a B1
comparison is only made on verified runs. Nothing is added to an analysis whose groups are all verified."""

ARMS = ("B1", "B1-roles")
SUFFIX = " (messaging unverified)"


def is_unverified(r):
    if r.get("arm") not in ARMS or not r.get("valid", True):
        return False
    mv = r.get("messaging_verified")
    return not (isinstance(mv, dict) and mv.get("status") == "verified")


def cell_fields(runs):
    """{"messaging_unverified": n} for a group with such runs, else {} (so an all-verified analysis is unchanged)."""
    n = sum(1 for r in runs if is_unverified(r))
    return {"messaging_unverified": n} if n else {}


def carry(cell):
    """The field a comparison row takes from its cell."""
    return {"messaging_unverified": cell["messaging_unverified"]} if cell.get("messaging_unverified") else {}


def label(arm, cell):
    return arm + SUFFIX if cell and cell.get("messaging_unverified") else arm


def labels_of(arms):
    """{arm: label} for the cells of one scenario."""
    return {arm: label(arm, c) for arm, c in arms.items()}


def relabel_tests(tests, arms):
    names = labels_of(arms)
    return [dict(t, arm=names.get(t["arm"], t["arm"]), against=names.get(t["against"], t["against"])) for t in tests]


def note_lines(a):
    """The note for the top of the report, or []: which scenarios and arms are labelled, and the rule."""
    hit = ["%s %s" % (sc, arm) for sc, s in a["scenarios"].items() for arm, c in s["arms"].items() if c.get("messaging_unverified")]
    if not hit:
        return []
    return ["**B1 messaging unverified**: %s include runs whose messaging_verified is not `verified` (SPEC 6.4), so they may be B0 "
            "plus a board. A B1 comparison is made only on verified runs; read these arms as unverified, not as a B1 result." %
            ", ".join(hit), ""]
