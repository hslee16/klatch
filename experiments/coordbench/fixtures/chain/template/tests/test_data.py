import os
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


class TestData(unittest.TestCase):
    def test_every_record_is_as_long_as_the_ruler_says(self):
        with open(os.path.join(ROOT, "data", "ruler.txt")) as f:
            total = sum(int(p.split("=")[1]) for p in f.read().strip().split(","))
        with open(os.path.join(ROOT, "data", "inventory.dat")) as f:
            lines = f.read().splitlines()
        self.assertTrue(lines)
        self.assertTrue(all(len(l) == total for l in lines))


if __name__ == "__main__":
    unittest.main()
