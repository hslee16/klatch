"""Integer ranges."""


def compress(numbers):
    """Sort the integers, drop duplicates and return the runs of consecutive values as a list of (low, high) tuples,
    both ends inclusive. compress([5, 1, 2, 3, 7, 7]) is [(1, 3), (5, 5), (7, 7)]. An empty input gives []."""
    raise NotImplementedError


def expand(ranges):
    """The inverse of compress: the sorted list of every integer in the (low, high) ranges (inclusive, duplicates
    removed). expand([(1, 3), (5, 5)]) is [1, 2, 3, 5]. A range with high below low raises ValueError."""
    raise NotImplementedError


def to_mask(ranges):
    """The 16 bit mask with one bit set for every integer in the ranges, using the repository's bit numbering (see
    docs/FORMATS.md). An integer outside 0 to 15 raises ValueError. to_mask([]) is 0."""
    raise NotImplementedError
