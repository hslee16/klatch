"""A small tokenizer."""
import re

_TOKEN = re.compile(r"\d+(?:\.\d+)?|[A-Za-z_][A-Za-z0-9_]*|\S")


def tokenize(text):
    """Split text into a list of tokens, skipping whitespace. A token is, in order of preference: a number (digits,
    optionally a dot and more digits), a word (a letter or underscore followed by letters, digits or underscores), or any
    other single non-space character. "x1 = 3.5;" gives ["x1", "=", "3.5", ";"]."""
    return _TOKEN.findall(text)


def token_kinds(text):
    """Like tokenize but returns a list of (token, kind) pairs where kind is "number", "word" or "symbol"."""
    out = []
    for tok in tokenize(text):
        kind = "number" if tok[0].isdigit() else "word" if (tok[0].isalpha() or tok[0] == "_") and tok.isascii() else "symbol"
        out.append((tok, kind))
    return out
