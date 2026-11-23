import unittest

from tallykit.matrix import identity, multiply, transpose


class TestMatrix(unittest.TestCase):
    def test_transpose(self):
        self.assertEqual(transpose([[1, 2, 3], [4, 5, 6]]), [[1, 4], [2, 5], [3, 6]])

    def test_multiply(self):
        self.assertEqual(multiply([[1, 2]], [[3], [4]]), [[11]])

    def test_identity(self):
        self.assertEqual(identity(2), [[1, 0], [0, 1]])
