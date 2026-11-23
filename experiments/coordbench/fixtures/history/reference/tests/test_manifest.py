import datetime
import unittest

from parcelkit.manifest import build_manifest, manifest_filename


class TestManifest(unittest.TestCase):
    def test_the_file_name_has_the_agreed_shape(self):
        self.assertRegex(manifest_filename(datetime.date(2026, 11, 20), 7), r"^[a-z0-9]+-20261120-007\.mf$")

    def test_rows_are_in_order_and_counted(self):
        m = build_manifest(["A1,900,12345", "B2,1500,90210"], datetime.date(2026, 11, 20), 1)
        self.assertEqual(m["count"], 2)
        self.assertEqual([r["ref"] for p in m["pages"] for r in p], ["A1", "B2"])
        self.assertEqual(m["total_cents"], sum(r["cents"] for p in m["pages"] for r in p))
