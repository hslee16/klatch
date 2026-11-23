import unittest

from tallykit.tokenizer import token_kinds, tokenize


class TestTokenizer(unittest.TestCase):
    def test_tokenize(self):
        self.assertEqual(tokenize("x1 = 3.5;"), ["x1", "=", "3.5", ";"])

    def test_kinds(self):
        self.assertEqual(token_kinds("a 2 +"), [("a", "word"), ("2", "number"), ("+", "symbol")])
