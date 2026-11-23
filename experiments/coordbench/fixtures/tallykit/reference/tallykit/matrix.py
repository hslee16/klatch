"""Small matrix helpers on lists of lists."""


def transpose(m):
    """The transpose of a matrix (a list of equal-length rows). transpose([]) is []."""
    return [list(col) for col in zip(*m)] if m else []


def multiply(a, b):
    """The matrix product a x b. When the number of columns of a differs from the number of rows of b, raise ValueError."""
    if not a or not b or len(a[0]) != len(b):
        raise ValueError("shapes do not match")
    cols = list(zip(*b))
    return [[sum(x * y for x, y in zip(row, col)) for col in cols] for row in a]


def identity(n):
    """The n by n identity matrix (n >= 0; identity(0) is []). A negative n raises ValueError."""
    if n < 0:
        raise ValueError("n must not be negative")
    return [[1 if i == j else 0 for j in range(n)] for i in range(n)]
