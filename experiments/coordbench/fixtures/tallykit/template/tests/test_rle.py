import unittest

from tallykit.rle import rle_decode, rle_encode


class TestRle(unittest.TestCase):
    def test_encode(self):
        self.assertEqual(rle_encode("aaabcc"), "3a1b2c")

    def test_decode(self):
        self.assertEqual(rle_decode("3a1b2c"), "aaabcc")
