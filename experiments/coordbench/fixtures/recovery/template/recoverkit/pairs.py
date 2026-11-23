"""Walking a sequence in groups."""


def pairs(seq):
    """The list of consecutive pairs of seq as tuples: pairs([1, 2, 3]) is [(1, 2), (2, 3)]. Fewer than two items give []."""
    raise NotImplementedError


def chunk(seq, n):
    """Cut seq into lists of n items; the last may be shorter. chunk([1, 2, 3, 4, 5], 2) is [[1, 2], [3, 4], [5]]. An n
    below 1 raises ValueError."""
    raise NotImplementedError


def window(seq, n):
    """Every run of n consecutive items as a list: window([1, 2, 3, 4], 3) is [[1, 2, 3], [2, 3, 4]]. Fewer than n items
    give []. An n below 1 raises ValueError."""
    raise NotImplementedError
