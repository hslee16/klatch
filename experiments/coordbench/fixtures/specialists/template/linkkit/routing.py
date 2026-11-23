"""Routing helpers (agent b)."""


def ledger_slot(n):
    """The ledger slot of record number n (an int, 0 or more): (n * n + 13) % LEDGER_MODULUS. LEDGER_MODULUS is defined in
    the ledger specification (held by agent c). Returns an int."""
    raise NotImplementedError


def frame_tag(kind):
    """The tag of a frame of the given kind (an int): (FRAME_MAGIC << 4) | (kind & 15). FRAME_MAGIC is defined in the
    framing specification (held by agent a). Returns an int."""
    raise NotImplementedError
