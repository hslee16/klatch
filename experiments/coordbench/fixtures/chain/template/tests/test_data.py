import os
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


class TestData(unittest.TestCase):
    def test_every_record_is_the_same_length(self):
        with open(os.path.join(ROOT, "data", "inventory.dat")) as f:
            lines = f.read().splitlines()
        self.assertTrue(lines)
        self.assertEqual(len({len(l) for l in lines}), 1)


if __name__ == "__main__":
    unittest.main()
