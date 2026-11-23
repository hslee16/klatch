"""A small tokenizer."""


def tokenize(text):
    """Split text into a list of tokens, skipping whitespace. A token is, in order of preference: a number (digits,
    optionally a dot and more digits), a word (a letter or underscore followed by letters, digits or underscores), or any
    other single non-space character. "x1 = 3.5;" gives ["x1", "=", "3.5", ";"]."""
    raise NotImplementedError


def token_kinds(text):
    """Like tokenize but returns a list of (token, kind) pairs where kind is "number", "word" or "symbol"."""
    raise NotImplementedError
