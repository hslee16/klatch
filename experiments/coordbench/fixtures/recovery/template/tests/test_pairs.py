import unittest

from recoverkit.pairs import chunk, pairs, window


class TestPairs(unittest.TestCase):
    def test_pairs(self):
        self.assertEqual(pairs([1, 2, 3]), [(1, 2), (2, 3)])

    def test_chunk_and_window(self):
        self.assertEqual(chunk([1, 2, 3], 2), [[1, 2], [3]])
        self.assertEqual(window([1, 2, 3], 2), [[1, 2], [2, 3]])
