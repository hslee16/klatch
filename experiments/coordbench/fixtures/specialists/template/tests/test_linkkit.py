import unittest

from linkkit import framing, ledger, retry, routing


class TestShapes(unittest.TestCase):
    """The visible tests only check types and ranges, so they pass with any value of a specification constant."""

    def test_framing(self):
        self.assertIsInstance(framing.topic_name(7), str)
        self.assertTrue(framing.topic_name(7).endswith(".0007"))
        self.assertIsInstance(framing.payload_digest(b"abc"), int)
        self.assertTrue(0 <= framing.payload_digest(b"abc") <= 0xFFFFFFFF)

    def test_retry(self):
        self.assertIsInstance(retry.retry_slot(3), int)
        self.assertTrue(retry.retry_slot(3) >= 0)

    def test_routing(self):
        self.assertIsInstance(routing.ledger_slot(5), int)
        self.assertEqual(routing.frame_tag(0x13) & 15, 3)

    def test_ledger(self):
        self.assertEqual(ledger.chunk_count(0), 0)
        self.assertIsInstance(ledger.shard_for("alpha"), int)
        self.assertTrue(ledger.shard_for("alpha") >= 0)


if __name__ == "__main__":
    unittest.main()
