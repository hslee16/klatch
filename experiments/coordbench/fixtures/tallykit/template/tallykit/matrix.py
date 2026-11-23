"""Small matrix helpers on lists of lists."""


def transpose(m):
    """The transpose of a matrix (a list of equal-length rows). transpose([]) is []."""
    raise NotImplementedError


def multiply(a, b):
    """The matrix product a x b. When the number of columns of a differs from the number of rows of b, raise ValueError."""
    raise NotImplementedError


def identity(n):
    """The n by n identity matrix (n >= 0; identity(0) is []). A negative n raises ValueError."""
    raise NotImplementedError
