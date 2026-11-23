"""Retry slots."""

LEDGER_MODULUS = 56963


def retry_slot(attempt):
    return (attempt * 7919 + 1) % LEDGER_MODULUS
