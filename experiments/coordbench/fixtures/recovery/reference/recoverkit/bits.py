"""Bit operations on 16 bit masks. Position 0 is the least significant bit (0x0001)."""


def _bit(index):
    if not isinstance(index, int) or not 0 <= index <= 15:
        raise ValueError("bit position %r is outside 0 to 15" % (index,))
    return 1 << index


def set_bit(mask, index):
    return mask | _bit(index)


def clear_bit(mask, index):
    return mask & ~_bit(index) & 0xFFFF


def test_bit(mask, index):
    return bool(mask & _bit(index))


def popcount(mask):
    return bin(mask & 0xFFFF).count("1")


def bit_indexes(mask):
    return [i for i in range(16) if mask & (1 << i)]
