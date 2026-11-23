"""URL slugs."""
import re
import unicodedata


def slugify(text, max_length=None):
    """Make a slug: normalise with NFKD and drop non-ASCII characters, lower-case, turn every run of characters outside
    a-z and 0-9 into one hyphen, strip hyphens from both ends. When max_length is given, cut to at most that many
    characters and strip a trailing hyphen again. slugify("Hello, World!") is "hello-world"."""
    ascii_text = unicodedata.normalize("NFKD", text).encode("ascii", "ignore").decode("ascii").lower()
    slug = re.sub(r"[^a-z0-9]+", "-", ascii_text).strip("-")
    if max_length is not None:
        slug = slug[:max_length].rstrip("-")
    return slug


def unique_slug(text, taken):
    """The slug of text, or when it is already in taken (any container), the slug with -2, -3, ... appended: the first
    free one. A text whose slug is empty uses "item" as the base."""
    base = slugify(text) or "item"
    if base not in taken:
        return base
    n = 2
    while "%s-%d" % (base, n) in taken:
        n += 1
    return "%s-%d" % (base, n)
