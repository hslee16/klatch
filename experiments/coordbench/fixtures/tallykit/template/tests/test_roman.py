import unittest

from tallykit.roman import from_roman, to_roman


class TestRoman(unittest.TestCase):
    def test_to_roman(self):
        self.assertEqual(to_roman(1994), "MCMXCIV")

    def test_from_roman(self):
        self.assertEqual(from_roman("XLII"), 42)

    def test_range(self):
        with self.assertRaises(ValueError):
            to_roman(0)
