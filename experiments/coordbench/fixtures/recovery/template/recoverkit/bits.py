"""Bit operations on 16 bit masks. An index here is a bit position: position 0 is the least significant bit (0x0001)."""


def set_bit(mask, index):
    """The mask with the bit at index set. An index outside 0 to 15 raises ValueError."""
    raise NotImplementedError


def clear_bit(mask, index):
    """The mask with the bit at index cleared. An index outside 0 to 15 raises ValueError."""
    raise NotImplementedError


def test_bit(mask, index):
    """True when the bit at index is set. An index outside 0 to 15 raises ValueError."""
    raise NotImplementedError


def popcount(mask):
    """How many bits of the 16 bit mask are set."""
    raise NotImplementedError


def bit_indexes(mask):
    """The positions of the set bits, in ascending order."""
    raise NotImplementedError
