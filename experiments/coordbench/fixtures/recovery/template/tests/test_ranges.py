import unittest

from recoverkit.ranges import compress, expand


class TestRanges(unittest.TestCase):
    def test_compress(self):
        self.assertEqual(compress([5, 1, 2, 3, 7, 7]), [(1, 3), (5, 5), (7, 7)])

    def test_expand(self):
        self.assertEqual(expand([(1, 3), (5, 5)]), [1, 2, 3, 5])
