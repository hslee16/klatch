import unittest

from recoverkit.wrap import unwrap, wrap


class TestWrap(unittest.TestCase):
    def test_wrap(self):
        self.assertEqual(wrap("aa bb cc", 5), ["aa bb", "cc"])

    def test_unwrap(self):
        self.assertEqual(unwrap(["aa bb", "cc"]), "aa bb cc")
