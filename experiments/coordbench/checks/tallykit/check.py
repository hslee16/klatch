"""Hidden checks for tallykit (#1366). Usage: check.py CHECKOUT. See check.sh for the output.

Each module is tested by loading its file directly (importlib, not through the package), so a broken registry cannot hide
a good module. The hidden tests are stronger than the visible ones: seeded random inputs against independent oracles,
edge cases and the documented errors. The seed is fixed so a check is repeatable.
"""
import ast
import datetime
import importlib.util
import json
import os
import random
import re
import subprocess
import sys
import unicodedata
import zlib

SEED = 20261122
MODULES = ["units", "tokenizer", "dates", "checksum", "slug", "roman", "rle", "matrix"]
EXPORTS = {
    "units": ["convert_length", "convert_temperature"],
    "tokenizer": ["tokenize", "token_kinds"],
    "dates": ["is_leap_year", "days_in_month", "days_between"],
    "checksum": ["adler32", "luhn_valid"],
    "slug": ["slugify", "unique_slug"],
    "roman": ["to_roman", "from_roman"],
    "rle": ["rle_encode", "rle_decode"],
    "matrix": ["transpose", "multiply", "identity"],
}
TICKETS = {"A": ["units", "tokenizer"], "B": ["dates", "checksum"], "C": ["slug", "roman"], "D": ["rle", "matrix"]}


def load(root, name):
    spec = importlib.util.spec_from_file_location("hidden_" + name, os.path.join(root, "tallykit", name + ".py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def raises(fn, exc, *args):
    try:
        fn(*args)
    except exc:
        return True
    except Exception:
        return False
    return False


def close(a, b):
    return isinstance(a, float) and abs(a - b) <= 1e-9 * max(1.0, abs(b))


def t_units(m):
    rng = random.Random(SEED + 1)
    factors = {"mm": 0.001, "cm": 0.01, "m": 1.0, "km": 1000.0, "in": 0.0254, "ft": 0.3048, "yd": 0.9144, "mi": 1609.344}
    for _ in range(200):
        a, b = rng.choice(list(factors)), rng.choice(list(factors))
        v = rng.uniform(-1e4, 1e4)
        if not close(m.convert_length(v, a, b), v * factors[a] / factors[b]):
            return False
    if not (close(m.convert_length(3, "ft", "in"), 36.0) and close(m.convert_length(1, "mi", "yd"), 1760.0)):
        return False
    for _ in range(200):
        a, b = rng.choice("CFK"), rng.choice("CFK")
        v = rng.uniform(-200, 500)
        k = v + 273.15 if a == "C" else (v - 32) * 5 / 9 + 273.15 if a == "F" else v
        want = k - 273.15 if b == "C" else (k - 273.15) * 9 / 5 + 32 if b == "F" else k
        if not close(m.convert_temperature(v, a, b), want):
            return False
    return (close(m.convert_temperature(-40, "C", "F"), -40.0) and close(m.convert_temperature(0, "K", "C"), -273.15)
            and raises(m.convert_length, ValueError, 1, "m", "furlong") and raises(m.convert_length, ValueError, 1, "x", "m")
            and raises(m.convert_temperature, ValueError, 1, "C", "R"))


def t_tokenizer(m):
    rng = random.Random(SEED + 2)
    cases = {"": [], "   ": [], "x1 = 3.5;": ["x1", "=", "3.5", ";"], "3.14.15": ["3.14", ".", "15"], "1a": ["1", "a"],
             "_a_b2+c": ["_a_b2", "+", "c"], "a,b": ["a", ",", "b"], "10": ["10"], "12.": ["12", "."], ".5": [".", "5"],
             "f(x)": ["f", "(", "x", ")"], "a  \t\n b": ["a", "b"]}
    for text, want in cases.items():
        if m.tokenize(text) != want:
            return False
    alphabet = "ab_Z09. +-*(\n\t"
    for _ in range(200):
        text = "".join(rng.choice(alphabet) for _ in range(rng.randint(0, 25)))
        want = re.findall(r"\d+(?:\.\d+)?|[A-Za-z_][A-Za-z0-9_]*|\S", text)
        if m.tokenize(text) != want:
            return False
        kinds = [(t, "number" if t[0].isdigit() else "word" if (t[0].isalpha() or t[0] == "_") else "symbol") for t in want]
        if m.token_kinds(text) != kinds:
            return False
    return True


def t_dates(m):
    rng = random.Random(SEED + 3)
    for y in [1900, 2000, 2023, 2024, 2100, 2400, 1, 4]:
        if m.is_leap_year(y) != (y % 4 == 0 and (y % 100 != 0 or y % 400 == 0)):
            return False
    for _ in range(100):
        y = rng.randint(1, 3000)
        for mo in range(1, 13):
            want = (datetime.date(y + (mo == 12), mo % 12 + 1, 1) - datetime.date(y, mo, 1)).days
            if m.days_in_month(y, mo) != want:
                return False
    if not (raises(m.days_in_month, ValueError, 2024, 0) and raises(m.days_in_month, ValueError, 2024, 13)):
        return False
    for _ in range(200):
        a = datetime.date(2000, 1, 1) + datetime.timedelta(days=rng.randint(0, 20000))
        b = datetime.date(2000, 1, 1) + datetime.timedelta(days=rng.randint(0, 20000))
        if m.days_between(a.isoformat(), b.isoformat()) != (b - a).days:
            return False
    return (m.days_between("2024-02-28", "2024-03-01") == 2 and m.days_between("2024-03-01", "2024-02-28") == -2
            and raises(m.days_between, ValueError, "2024-02-30", "2024-03-01")
            and raises(m.days_between, ValueError, "2024-01-01", "not a date"))


def t_checksum(m):
    rng = random.Random(SEED + 4)
    if m.adler32(b"") != 1 or m.adler32(b"Wikipedia") != 0x11E60398:
        return False
    for _ in range(100):
        data = bytes(rng.randrange(256) for _ in range(rng.randint(0, 6000)))
        if m.adler32(data) != zlib.adler32(data):
            return False
    if m.adler32(b"\xff" * 100000) != zlib.adler32(b"\xff" * 100000):
        return False

    def luhn(s):
        tot = 0
        for i, ch in enumerate(reversed(s)):
            d = int(ch) * (2 if i % 2 else 1)
            tot += d - 9 if d > 9 else d
        return tot % 10 == 0
    for _ in range(300):
        s = "".join(rng.choice("0123456789") for _ in range(rng.randint(1, 19)))
        spaced = " ".join(s[i:i + 4] for i in range(0, len(s), 4))
        if m.luhn_valid(s) is not luhn(s) or m.luhn_valid(spaced) is not luhn(s):
            return False
    return (m.luhn_valid("79927398713") is True and m.luhn_valid("79927398710") is False and m.luhn_valid("") is False
            and m.luhn_valid("   ") is False and m.luhn_valid("7992a398713") is False and m.luhn_valid("0") is True)


def t_slug(m):
    def slugify(text, n=None):
        t = unicodedata.normalize("NFKD", text).encode("ascii", "ignore").decode("ascii").lower()
        s = re.sub(r"[^a-z0-9]+", "-", t).strip("-")
        return s if n is None else s[:n].rstrip("-")
    fixed = ["Hello, World!", "  --A  B--  ", "Crème brûlée", "Ünïcode", "", "!!!", "a_b c", "100% sure", "日本語", "x" * 40]
    for text in fixed:
        if m.slugify(text) != slugify(text):
            return False
    rng = random.Random(SEED + 5)
    alphabet = "abcXYZ019 -_,.!éü\t"
    for _ in range(300):
        text = "".join(rng.choice(alphabet) for _ in range(rng.randint(0, 30)))
        n = rng.choice([None, 0, 1, 3, 5, 10, 50])
        if m.slugify(text, n) != slugify(text, n) or m.slugify(text) != slugify(text):
            return False
    if m.slugify("Hello, World!", max_length=7) != "hello-w" or m.slugify("ab cd", max_length=3) != "ab":
        return False
    return (m.unique_slug("Hello", set()) == "hello" and m.unique_slug("Hello", {"hello"}) == "hello-2"
            and m.unique_slug("Hello", ["hello", "hello-2", "hello-3"]) == "hello-4"
            and m.unique_slug("Hello", {"hello", "hello-3"}) == "hello-2" and m.unique_slug("!!", set()) == "item"
            and m.unique_slug("!!", {"item"}) == "item-2")


def t_roman(m):
    table = [(1000, "M"), (900, "CM"), (500, "D"), (400, "CD"), (100, "C"), (90, "XC"), (50, "L"), (40, "XL"), (10, "X"),
             (9, "IX"), (5, "V"), (4, "IV"), (1, "I")]

    def oracle(n):
        out = ""
        for v, s in table:
            out += s * (n // v)
            n %= v
        return out
    for n in range(1, 4000):
        r = m.to_roman(n)
        if r != oracle(n) or m.from_roman(r) != n:
            return False
    known = {1: "I", 4: "IV", 9: "IX", 14: "XIV", 40: "XL", 90: "XC", 400: "CD", 1994: "MCMXCIV", 2024: "MMXXIV",
             3999: "MMMCMXCIX"}
    if any(m.to_roman(n) != s for n, s in known.items()):
        return False
    for n in (0, -1, 4000, 10 ** 6):
        if not raises(m.to_roman, ValueError, n):
            return False
    for s in ("", "IIII", "VX", "IC", "iv", "MMMM", "IIV", "XXXX", "VV", "IXI", "A", "MCMXCIVX", "CMC"):
        if not raises(m.from_roman, ValueError, s):
            return False
    return True


def t_rle(m):
    rng = random.Random(SEED + 7)
    if (m.rle_encode("") != "" or m.rle_encode("aaabcc") != "3a1b2c" or m.rle_encode("a") != "1a"
            or m.rle_encode("aaaaaaaaaaaa") != "12a" or m.rle_encode("  \n\n") != "2 2\n"):
        return False
    if m.rle_decode("") != "" or m.rle_decode("3a1b2c") != "aaabcc" or m.rle_decode("12a") != "a" * 12:
        return False
    for _ in range(300):
        text = "".join(rng.choice("aabbc -z") * rng.randint(1, 15) for _ in range(rng.randint(0, 8)))
        enc = m.rle_encode(text)
        want = "".join("%d%s" % (len(g.group(0)), g.group(1)) for g in re.finditer(r"(.)\1*", text, re.S))
        if enc != want or m.rle_decode(enc) != text:
            return False
    return (raises(m.rle_encode, ValueError, "a1b") and raises(m.rle_decode, ValueError, "3")
            and raises(m.rle_decode, ValueError, "abc") and raises(m.rle_decode, ValueError, "0a")
            and raises(m.rle_decode, ValueError, "3a2"))


def t_matrix(m):
    rng = random.Random(SEED + 8)
    if m.transpose([]) != [] or m.transpose([[1, 2, 3]]) != [[1], [2], [3]] or m.identity(0) != [] or m.identity(1) != [[1]]:
        return False
    for _ in range(100):
        r, c, k = rng.randint(1, 5), rng.randint(1, 5), rng.randint(1, 5)
        a = [[rng.randint(-9, 9) for _ in range(c)] for _ in range(r)]
        b = [[rng.randint(-9, 9) for _ in range(k)] for _ in range(c)]
        want = [[sum(a[i][x] * b[x][j] for x in range(c)) for j in range(k)] for i in range(r)]
        if m.multiply(a, b) != want:
            return False
        t = m.transpose(a)
        if len(t) != c or len(t[0]) != r or any(t[j][i] != a[i][j] for i in range(r) for j in range(c)):
            return False
        if m.transpose(t) != a or m.multiply(m.identity(r), a) != a or m.multiply(a, m.identity(c)) != a:
            return False
    orig = [[1, 2], [3, 4]]
    snapshot = [row[:] for row in orig]
    m.transpose(orig)
    m.multiply(orig, orig)
    return (orig == snapshot and raises(m.multiply, ValueError, [[1, 2]], [[1, 2]])
            and raises(m.multiply, ValueError, [[1, 2, 3]], [[1], [2]]) and raises(m.identity, ValueError, -1)
            and m.identity(3) == [[1, 0, 0], [0, 1, 0], [0, 0, 1]])


TESTS = {"units": t_units, "tokenizer": t_tokenizer, "dates": t_dates, "checksum": t_checksum, "slug": t_slug,
         "roman": t_roman, "rle": t_rle, "matrix": t_matrix}


def module_ok(root, name):
    try:
        return bool(TESTS[name](load(root, name)))
    except BaseException:  # NotImplementedError, SyntaxError, anything an agent's module can raise
        return False


def tracked_files(root):
    p = subprocess.run(["git", "ls-files"], cwd=root, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    return [f for f in p.stdout.splitlines() if f] if p.returncode == 0 else []


def has_conflict_markers(root):
    marker = re.compile(r"^(<<<<<<< |=======$|>>>>>>> )")
    for f in tracked_files(root) or []:
        if f == "TASKS.md":  # B1's board: git merges it with merge=union, so markers there are not code (#1523)
            continue
        path = os.path.join(root, f)
        try:
            with open(path, encoding="utf-8", errors="replace") as fh:
                if any(marker.match(line) for line in fh):
                    return True
        except OSError:
            pass
    return False


def registry_lines(root):
    """{module: [names]} from the import lines of the package's __init__.py, or None when the file is missing, does not parse
    or has two lines for one module."""
    try:
        with open(os.path.join(root, "tallykit", "__init__.py")) as f:
            tree = ast.parse(f.read())
    except (OSError, SyntaxError, ValueError):
        return None
    seen = {}
    for mod, names in import_lines(tree, "tallykit."):
        if mod in seen:
            return None
        seen[mod] = names
    return seen


def import_lines(tree, prefix):
    """(module, sorted names) of each top level `from prefix.module import names` statement."""
    for node in tree.body:
        if not isinstance(node, ast.ImportFrom):
            continue
        mod = node.module or ""
        if mod.startswith(prefix):
            yield mod[len(prefix):], sorted(a.name for a in node.names)


def names_cover(seen, wanted):
    """The registry has a line for exactly the wanted modules and each line names at least the wanted functions."""
    if set(seen) != set(wanted):
        return False
    return all(set(fs) <= set(seen[m]) for m, fs in wanted.items())


def registry_ok(root):
    """tallykit/__init__.py imports and exports every function, with exactly one registry line per module. A line may also
    name a public helper of its module (the registry text asks for every public function, #1525): the names the checker needs
    must be there and every name on a line must import."""
    seen = registry_lines(root)
    if seen is None or not names_cover(seen, EXPORTS):
        return False
    code = ("import tallykit, json; names = %r; print(json.dumps(all(callable(getattr(tallykit, n, None)) for n in names)))"
            % sorted({n for names in seen.values() for n in names}))
    p = subprocess.run([sys.executable, "-c", code], cwd=root, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
                       timeout=60)
    return p.returncode == 0 and p.stdout.strip() == "true"


def changelog_ok(root):
    try:
        with open(os.path.join(root, "CHANGELOG.md")) as f:
            lines = f.read().splitlines()
    except OSError:
        return False
    entries = [l for l in lines if re.match(r"^- ticket [A-Z]:", l)]
    want = sorted("- ticket %s: %s" % (t, ", ".join(ms)) for t, ms in TICKETS.items())
    return sorted(entries) == want


def visible_tests_ok(root):
    try:
        p = subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", "tests", "-t", "."], cwd=root,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=90)
    except subprocess.TimeoutExpired:
        return False
    return p.returncode == 0


def per_agent(root):
    """Commits and modules touched, by git author, skipping the root commit and merge commits."""
    p = subprocess.run(["git", "log", "--no-merges", "--no-renames", "--format=%x01%an%x01%P", "--name-only"], cwd=root,
                       stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    out = {}
    parts = p.stdout.split("\x01") if p.returncode == 0 else []
    for i in range(1, len(parts) - 1, 2):
        author, rest = parts[i], parts[i + 1]
        parents, _, files = rest.partition("\n")
        if not parents.strip():
            continue  # the root commit is the harness's initial tree
        a = out.setdefault(author, {"commits": 0, "modules": []})
        a["commits"] += 1
        for fn in files.split():
            m = re.fullmatch(r"tallykit/(\w+)\.py", fn)
            if m and m.group(1) in MODULES and m.group(1) not in a["modules"]:
                a["modules"].append(m.group(1))
    for a in out.values():
        a["modules"].sort()
    return out


def main():
    root = os.path.abspath(sys.argv[1])
    checks = [{"id": m, "pass": module_ok(root, m)} for m in MODULES]
    units = sum(1 for c in checks if c["pass"])
    markers = has_conflict_markers(root)
    reg, chg, vis = registry_ok(root), changelog_ok(root), visible_tests_ok(root)
    checks += [{"id": "registry", "unit": False, "pass": reg}, {"id": "changelog", "unit": False, "pass": chg},
               {"id": "no-conflict-markers", "unit": False, "pass": not markers},
               {"id": "visible-tests", "unit": False, "pass": vis}]
    merge_state = "conflicts-left" if markers else ("clean" if vis else "tests-broken")
    correct = units == len(MODULES) and reg and chg and not markers and vis
    print(json.dumps({"checks": checks, "units_completed": units, "units_total": len(MODULES), "merge_state": merge_state,
                      "correct": correct, "per_agent": per_agent(root)}, sort_keys=True))


if __name__ == "__main__":
    main()
