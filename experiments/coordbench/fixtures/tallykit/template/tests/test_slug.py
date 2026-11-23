import unittest

from tallykit.slug import slugify, unique_slug


class TestSlug(unittest.TestCase):
    def test_slugify(self):
        self.assertEqual(slugify("Hello, World!"), "hello-world")

    def test_unique(self):
        self.assertEqual(unique_slug("Hello", {"hello"}), "hello-2")
