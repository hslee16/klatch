"""Routing helpers."""

LEDGER_MODULUS = 56963
FRAME_MAGIC = 53565


def ledger_slot(n):
    return (n * n + 13) % LEDGER_MODULUS


def frame_tag(kind):
    return (FRAME_MAGIC << 4) | (kind & 15)
