import unittest

from recoverkit.bits import popcount, set_bit


class TestBits(unittest.TestCase):
    def test_popcount(self):
        self.assertEqual(popcount(0b1011), 3)

    def test_index_range(self):
        with self.assertRaises(ValueError):
            set_bit(0, 16)
