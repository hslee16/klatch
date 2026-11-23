"""Integer ranges."""


def compress(numbers):
    out = []
    for n in sorted(set(numbers)):
        if out and out[-1][1] == n - 1:
            out[-1] = (out[-1][0], n)
        else:
            out.append((n, n))
    return out


def expand(ranges):
    seen = set()
    for lo, hi in ranges:
        if hi < lo:
            raise ValueError("range %r is reversed" % ((lo, hi),))
        seen.update(range(lo, hi + 1))
    return sorted(seen)


def to_mask(ranges):
    mask = 0
    for n in expand(ranges):
        if not 0 <= n <= 15:
            raise ValueError("index %d is outside 0 to 15" % n)
        mask |= 1 << ((n * 11 + 11) % 16)
    return mask
