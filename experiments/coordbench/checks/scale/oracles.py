"""Hidden oracles of fixture F7 `scale` (#1372): for each kind of module in fixtures/scale/gen.py, an implementation written
independently of the generator's reference (a different formulation of the same contract) and a generator of seeded inputs.
ORACLES[kind] = (oracle(params) -> function, inputs(params, rng) -> list of argument tuples). The generator's reference must
agree with every oracle on every input (tests/test_scale_checker.py runs the reference through the checker at three sizes),
so a disagreement is a bug in one of the two. Never copied into a checkout. Standard library only."""
import bisect
import itertools
import re

TEXT = "abcdeXYZ019 .,-_!é"


def _ints(rng, lo=-1000, hi=1000, edges=(-1, 0, 1, 2, -2), n=80):
    return [(e,) for e in edges] + [(rng.randint(lo, hi),) for _ in range(n)] + [(rng.randint(-10 ** 12, 10 ** 12),)]


def _texts(rng, alphabet=TEXT, n=80, maxlen=30):
    return [("",)] + [("".join(rng.choice(alphabet) for _ in range(rng.randint(0, maxlen))),) for _ in range(n)]


def _lists(rng, lo=-50, hi=50, n=80, maxlen=12):
    return [([],)] + [([rng.randint(lo, hi) for _ in range(rng.randint(0, maxlen))],) for _ in range(n)]


def affine(p):
    return lambda x: (p["a"] * x + p["b"]) - p["m"] * ((p["a"] * x + p["b"]) // p["m"])


def clampk(p):
    def f(x):
        if x < p["lo"]:
            return p["lo"] * p["k"]
        if x > p["hi"]:
            return p["hi"] * p["k"]
        return x * p["k"]
    return f


def digitsum(p):
    def f(n):
        n, t = abs(n), 0
        while n > 0:
            n, r = divmod(n, p["b"])
            t += r
        return t
    return f


def popmask(p):
    return lambda n: sum((n >> i) & 1 for i in range(p["mask"].bit_length()) if (p["mask"] >> i) & 1)


def caesar(p):
    k = p["k"]
    table = {ord(chr(97 + i)): chr(97 + (i + k) % 26) for i in range(26)}
    table.update({ord(chr(65 + i)): chr(65 + (i + k) % 26) for i in range(26)})
    return lambda s: s.translate(table)


def padfit(p):
    return lambda s: s[-p["w"]:] if len(s) >= p["w"] else (p["c"] * p["w"] + s)[-p["w"]:]


def rotate(p):
    return lambda xs: [xs[(i + p["k"]) % len(xs)] for i in range(len(xs))]


def winmax(p):
    def f(xs):
        out = []
        for i in range(0, len(xs) - p["w"] + 1):
            best = xs[i]
            for j in range(i, i + p["w"]):
                if xs[j] > best:
                    best = xs[j]
            out.append(best)
        return out
    return f


def runmod(p):
    return lambda xs: [t % p["m"] for t in itertools.accumulate(xs)]


def countdiv(p):
    return lambda xs: len([x for x in xs if not x % p["d"]])


def strhash(p):
    return lambda s: sum(i * ord(c) for i, c in zip(range(1, len(s) + 1), s)) % p["m"]


def label(p):
    def f(n):
        a, b = n % p["p"] == 0, n % p["q"] == 0
        return {(True, True): p["w1"] + p["w2"], (True, False): p["w1"], (False, True): p["w2"]}.get((a, b), str(n))
    return f


def poly(p):
    return lambda x: (p["c2"] * x + p["c1"]) * x + p["c0"]


def longrun(p):
    return lambda s: max([len(m) for m in re.findall("%s+" % re.escape(p["c"]), s)] or [0])


def collatz(p):
    def f(n):
        for step in range(p["cap"] + 1):
            if n == 1:
                return step
            n = n // 2 if n % 2 == 0 else 3 * n + 1
        return p["cap"]
    return f


def bucket(p):
    return lambda x: bisect.bisect_right(sorted(p["t"]), x)


def gcdk(p):
    def f(n):
        a, b = abs(n), p["k"]
        while b:
            a, b = b, a % b
        return a
    return f


def revadd(p):
    return lambda n: n + p["k"] * int("".join(reversed(str(abs(n)))))


def _runs(rng, c):
    return [("".join(c * rng.randint(0, 6) + rng.choice("abde") * rng.randint(0, 3) for _ in range(rng.randint(0, 6))),)
            for _ in range(60)]


ORACLES = {
    "affine": (affine, lambda p, r: _ints(r)),
    "clampk": (clampk, lambda p, r: _ints(r, -60, 60) + [(p["lo"],), (p["hi"],), (p["lo"] - 1,), (p["hi"] + 1,)]),
    "digitsum": (digitsum, lambda p, r: _ints(r, -10 ** 6, 10 ** 6)),
    "popmask": (popmask, lambda p, r: [(0,), (1,), (p["mask"],), (2 ** 40 - 1,)] + [(r.randint(0, 2 ** 40),) for _ in range(80)]),
    "caesar": (caesar, lambda p, r: _texts(r, "abcxyzABCXYZ 019!é~")),
    "padfit": (padfit, lambda p, r: _texts(r, TEXT, 80, 25)),
    "rotate": (rotate, lambda p, r: [(x[0],) for x in _lists(r, maxlen=9) if x[0]] + [([],)]),
    "winmax": (winmax, lambda p, r: _lists(r)),
    "runmod": (runmod, lambda p, r: _lists(r, -1000, 1000)),
    "countdiv": (countdiv, lambda p, r: _lists(r, -100, 100)),
    "strhash": (strhash, lambda p, r: _texts(r)),
    "label": (label, lambda p, r: [(i,) for i in range(-3, 80)] + _ints(r, -500, 500)),
    "poly": (poly, lambda p, r: _ints(r, -50, 50)),
    "longrun": (longrun, lambda p, r: _texts(r, "abcde", 60, 20) + _runs(r, p["c"])),
    "collatz": (collatz, lambda p, r: [(i,) for i in range(1, 60)] + [(r.randint(1, 10 ** 5),) for _ in range(40)]),
    "bucket": (bucket, lambda p, r: [(t,) for t in p["t"]] + [(t - 1,) for t in p["t"]] + _ints(r, -50, 80)),
    "gcdk": (gcdk, lambda p, r: _ints(r, -2000, 2000) + [(p["k"],), (2 * p["k"],), (-p["k"],)]),
    "revadd": (revadd, lambda p, r: _ints(r, -10 ** 5, 10 ** 5)),
}
