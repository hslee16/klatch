"""Statistics for the coordination bench, Python 3 standard library only (#1364, SPEC section 6).

scipy is not available, so the tests the spec names are implemented here: Fisher exact (exact enumeration over the
hypergeometric), Mann-Whitney U (exact distribution when there are no ties and the groups are small, the normal
approximation with tie correction otherwise), the Hodges-Lehmann shift, a seeded percentile bootstrap interval for the
difference in medians, Wilson and Newcombe intervals, and the Holm correction. Each is tested against values known from
the literature or worked by hand in tests/test_stats.py.
"""
import math
import random

Z95 = 1.959964  # the 97.5th percentile of the standard normal
EXACT_MAX_GROUP = 20  # the exact Mann-Whitney distribution is used up to this many values per group


def quantile(values, q):
    """The q quantile by linear interpolation between order statistics (the R type 7 rule)."""
    v = sorted(values)
    if not v:
        return None
    pos = (len(v) - 1) * q
    lo = int(math.floor(pos))
    hi = min(lo + 1, len(v) - 1)
    return v[lo] + (v[hi] - v[lo]) * (pos - lo)


def median(values):
    return quantile(values, 0.5)


def summary(values):
    """N, median, quartiles, min, max, then mean and sample sd as a second line (SPEC 6.2). None for no values."""
    v = [float(x) for x in values]
    if not v:
        return None
    n = len(v)
    mean = sum(v) / n
    sd = math.sqrt(sum((x - mean) ** 2 for x in v) / (n - 1)) if n > 1 else None
    return {"n": n, "median": median(v), "q1": quantile(v, 0.25), "q3": quantile(v, 0.75), "min": min(v),
            "max": max(v), "mean": mean, "sd": sd}


def wilson(k, n, z=Z95):
    """The Wilson score interval for k successes in n trials, or (None, None) for n = 0."""
    if n <= 0:
        return (None, None)
    p = k / n
    z2 = z * z
    denom = 1 + z2 / n
    centre = (p + z2 / (2 * n)) / denom
    half = z * math.sqrt(p * (1 - p) / n + z2 / (4 * n * n)) / denom
    return (max(0.0, centre - half), min(1.0, centre + half))


def newcombe_diff(k1, n1, k2, n2, z=Z95):
    """Newcombe's hybrid score interval (method 10) for the difference of two proportions p1 - p2."""
    if n1 <= 0 or n2 <= 0:
        return (None, None, None)
    p1, p2 = k1 / n1, k2 / n2
    l1, u1 = wilson(k1, n1, z)
    l2, u2 = wilson(k2, n2, z)
    d = p1 - p2
    return (d, d - math.sqrt((p1 - l1) ** 2 + (u2 - p2) ** 2), d + math.sqrt((u1 - p1) ** 2 + (p2 - l2) ** 2))


def risk_ratio(k1, n1, k2, n2):
    """(k1/n1) / (k2/n2), or None when the second rate is zero or a group is empty."""
    if n1 <= 0 or n2 <= 0 or k2 == 0:
        return None
    return (k1 / n1) / (k2 / n2)


def fisher_exact(a, b, c, d, alternative="two-sided"):
    """Fisher's exact test on the table [[a, b], [c, d]] by enumerating the hypergeometric distribution.

    two-sided sums the probability of every table (with the same margins) that is no more likely than the observed
    one; "greater" is P(X >= a) and "less" is P(X <= a), X being the top-left cell.
    """
    if min(a, b, c, d) < 0:
        raise ValueError("counts cannot be negative")
    row1, row2, col1 = a + b, c + d, a + c
    total = row1 + row2
    if total == 0 or row1 == 0 or row2 == 0 or col1 == 0 or col1 == total:
        return 1.0
    denom = math.comb(total, col1)
    lo, hi = max(0, col1 - row2), min(row1, col1)

    def prob(x):
        return math.comb(row1, x) * math.comb(row2, col1 - x) / denom

    if alternative == "greater":
        return min(1.0, sum(prob(x) for x in range(a, hi + 1)))
    if alternative == "less":
        return min(1.0, sum(prob(x) for x in range(lo, a + 1)))
    if alternative != "two-sided":
        raise ValueError("alternative must be two-sided, greater or less")
    p_obs = prob(a)
    return min(1.0, sum(p for p in (prob(x) for x in range(lo, hi + 1)) if p <= p_obs * (1 + 1e-7)))


def _midranks(values):
    """Average ranks (1-based) of the values, and the tie group sizes."""
    order = sorted(range(len(values)), key=lambda i: values[i])
    ranks = [0.0] * len(values)
    ties = []
    i = 0
    while i < len(order):
        j = i
        while j + 1 < len(order) and values[order[j + 1]] == values[order[i]]:
            j += 1
        r = (i + j) / 2 + 1
        for k in range(i, j + 1):
            ranks[order[k]] = r
        ties.append(j - i + 1)
        i = j + 1
    return ranks, ties


def _exact_u_counts(n, m):
    """counts[u] = the number of the C(n+m, n) assignments of ranks to the first group that give U = u."""
    big_n = n + m
    # ways[j][s]: choose j of the ranks 1..N with rank sum s
    max_s = big_n * (big_n + 1) // 2
    ways = [[0] * (max_s + 1) for _ in range(n + 1)]
    ways[0][0] = 1
    for rank in range(1, big_n + 1):
        for j in range(min(n, rank), 0, -1):
            row, prev = ways[j], ways[j - 1]
            for s in range(max_s, rank - 1, -1):
                if prev[s - rank]:
                    row[s] += prev[s - rank]
    base = n * (n + 1) // 2
    return {s - base: c for s, c in enumerate(ways[n]) if c and s >= base}


def mann_whitney(x, y):
    """Two-sided Mann-Whitney U test of x against y.

    Returns {"u1", "u2", "u", "p", "method", "z"}. method is "exact" (no ties, at most 20 values per group, the exact
    null distribution), "normal-ties" (the normal approximation with tie correction and a 0.5 continuity correction)
    or "normal" (no ties but large groups). None when a group is empty.
    """
    n, m = len(x), len(y)
    if n == 0 or m == 0:
        return None
    combined = [float(v) for v in list(x) + list(y)]
    ranks, ties = _midranks(combined)
    r1 = sum(ranks[:n])
    u1 = r1 - n * (n + 1) / 2
    u2 = n * m - u1
    out = {"u1": u1, "u2": u2, "u": min(u1, u2), "z": None}
    has_ties = any(t > 1 for t in ties)
    if not has_ties and n <= EXACT_MAX_GROUP and m <= EXACT_MAX_GROUP:
        counts = _exact_u_counts(n, m)
        total = math.comb(n + m, n)
        centre = n * m / 2
        dev = abs(u1 - centre)
        p = sum(c for u, c in counts.items() if abs(u - centre) >= dev - 1e-9) / total
        out.update(p=min(1.0, p), method="exact")
        return out
    big_n = n + m
    tie_term = sum(t ** 3 - t for t in ties)
    var = n * m / 12 * ((big_n + 1) - tie_term / (big_n * (big_n - 1)))
    out["method"] = "normal-ties" if has_ties else "normal"
    if var <= 0:
        out.update(p=1.0, z=0.0)
        return out
    z = max(0.0, abs(u1 - n * m / 2) - 0.5) / math.sqrt(var)
    out.update(p=min(1.0, math.erfc(z / math.sqrt(2))), z=z)
    return out


def hodges_lehmann(a, b):
    """The Hodges-Lehmann shift of b relative to a: the median of all pairwise differences b_j - a_i."""
    return median([float(bj) - float(ai) for ai in a for bj in b])


def bootstrap_median_diff_ci(a, b, resamples=10000, seed=0, alpha=0.05):
    """Percentile bootstrap interval for median(b) - median(a), resampling each group with replacement.

    The generator is random.Random(seed), so a recorded seed reproduces the interval exactly.
    """
    if not a or not b:
        return (None, None)
    rng = random.Random(seed)
    na, nb = len(a), len(b)
    diffs = []
    for _ in range(resamples):
        ra = [a[rng.randrange(na)] for _ in range(na)]
        rb = [b[rng.randrange(nb)] for _ in range(nb)]
        diffs.append(median(rb) - median(ra))
    return (quantile(diffs, alpha / 2), quantile(diffs, 1 - alpha / 2))


def holm(pvalues):
    """Holm-Bonferroni adjusted p values, in the order given."""
    m = len(pvalues)
    order = sorted(range(m), key=lambda i: pvalues[i])
    adj = [0.0] * m
    running = 0.0
    for rank, i in enumerate(order):
        running = max(running, (m - rank) * pvalues[i])
        adj[i] = min(1.0, running)
    return adj
