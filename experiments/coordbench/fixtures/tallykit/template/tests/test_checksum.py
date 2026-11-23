import unittest

from tallykit.checksum import adler32, luhn_valid


class TestChecksum(unittest.TestCase):
    def test_adler(self):
        self.assertEqual(adler32(b"Wikipedia"), 0x11E60398)

    def test_luhn(self):
        self.assertTrue(luhn_valid("4539 1488 0343 6467"))
        self.assertFalse(luhn_valid("4539 1488 0343 6468"))
