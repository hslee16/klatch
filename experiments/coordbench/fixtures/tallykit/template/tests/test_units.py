import unittest

from tallykit.units import convert_length, convert_temperature


class TestUnits(unittest.TestCase):
    def test_length(self):
        self.assertAlmostEqual(convert_length(1, "km", "m"), 1000.0)
        self.assertAlmostEqual(convert_length(12, "in", "ft"), 1.0)

    def test_temperature(self):
        self.assertAlmostEqual(convert_temperature(100, "C", "F"), 212.0)

    def test_unknown_unit(self):
        with self.assertRaises(ValueError):
            convert_length(1, "parsec", "m")
