import unittest

from tallykit.dates import days_between, days_in_month, is_leap_year


class TestDates(unittest.TestCase):
    def test_leap(self):
        self.assertTrue(is_leap_year(2024))
        self.assertFalse(is_leap_year(2023))

    def test_days_in_month(self):
        self.assertEqual(days_in_month(2024, 2), 29)
        self.assertEqual(days_in_month(2023, 4), 30)

    def test_between(self):
        self.assertEqual(days_between("2024-01-01", "2024-01-31"), 30)
