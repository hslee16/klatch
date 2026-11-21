import unicodedata
import re


def slugify(title: str, sep: str = "-", max_len: int = 80) -> str:
    """Lowercase ASCII slug. Accents are folded (Cafe for Café); runs of other characters become one sep."""
    folded = unicodedata.normalize("NFKD", title).encode("ascii", "ignore").decode("ascii")
    slug = re.sub(r"[^a-z0-9]+", sep, folded.lower()).strip(sep)
    return slug[:max_len].rstrip(sep)
