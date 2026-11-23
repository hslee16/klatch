"""Word wrapping."""


def wrap(text, width):
    """Split text into lines of at most width characters, breaking at single spaces, greedily: a word goes on the current
    line when the line and the word and one space still fit. A word longer than width gets a line of its own, whole.
    Runs of whitespace count as one space. Empty text gives []. A width below 1 raises ValueError."""
    raise NotImplementedError


def unwrap(lines):
    """Join lines back into one text with single spaces (the inverse of wrap up to whitespace). unwrap([]) is ''."""
    raise NotImplementedError
