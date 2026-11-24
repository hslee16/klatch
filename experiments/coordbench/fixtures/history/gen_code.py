"""The text of the files of fixture F9 `history` (#1374), for gen_facts.py: the code and tests of the six finished tickets, by
version (no seeded value appears in any of it), the README, and the reference manifest module with its test. Standard
library only. A generator source: its output is committed and checked by `gen_facts.py --check`, so it is not in the fixture
hash (fixture.py)."""

# --- the code of the six finished tickets, by version (no seeded value appears in any of it) ---------------------------

INIT = '"""parcelkit: shipping manifests for a small warehouse."""\n'

WEIGHTS = '''"""Weights. Inside parcelkit every weight is a whole number of grams."""


def to_grams(kg):
    """Kilograms (a float) to whole grams, rounded to the nearest gram."""
    return int(round(kg * 1000))


def block_count(weight_g, block_g=100):
    """How many started blocks of block_g grams the weight fills (0 for a weight of 0 or less)."""
    if weight_g <= 0:
        return 0
    return -(-weight_g // block_g)
'''

ZONES_V1 = '''"""Postage zones from postcodes."""


def zone_for(postcode):
    """The zone of a five digit postcode: first digit 0-2 is zone 1, 3-5 is zone 2, anything else is zone 3."""
    first = int(postcode[0])
    if first <= 2:
        return 1
    if first <= 5:
        return 2
    return 3
'''
ZONES_V2 = ZONES_V1.replace('''anything else is zone 3."""''', '''6-8 is zone 3, 9 is zone 4 (remote)."""''').replace(
    "    return 3\n", "    if first <= 8:\n        return 3\n    return 4\n")
ZONES_V3 = ZONES_V2.replace('''    first = int(postcode[0])''', '''    if len(postcode) != 5 or not postcode.isdigit():
        raise ValueError("not a postcode: %r" % (postcode,))
    first = int(postcode[0])''')

RATES_V1 = '''"""Prices."""

BASE_CENTS = {1: 450, 2: 620, 3: 810, 4: 1100}


def rate_cents(net_weight_g, zone):
    """Price in cents of one parcel in a zone. net_weight_g is the NET weight in whole grams."""
    return BASE_CENTS[zone]
'''
RATES_V2 = '''"""Prices."""
from parcelkit.weights import block_count

BASE_CENTS = {1: 450, 2: 620, 3: 810, 4: 1100}
BLOCK_CENTS = {1: 35, 2: 48, 3: 62, 4: 90}


def rate_cents(net_weight_g, zone):
    """Price in cents of one parcel in a zone: the zone's base plus its price for every started 100 g block of the NET
    weight (whole grams)."""
    return BASE_CENTS[zone] + BLOCK_CENTS[zone] * block_count(net_weight_g)
'''
RATES_V3 = RATES_V2.replace('''    return BASE_CENTS[zone]''', '''    if net_weight_g < 0:
        raise ValueError("a net weight cannot be negative")
    return BASE_CENTS[zone]''')

PARCELS_V1 = '''"""Parcels as they arrive from the scanner."""


def parse_line(line):
    """Read 'ref,weight_g,postcode' into {"ref", "weight_g" (an int), "postcode" (a string)}; ValueError when the line
    is not that."""
    parts = line.strip().split(",")
    if len(parts) != 3 or not parts[0]:
        raise ValueError("expected ref,weight_g,postcode: %r" % (line,))
    return {"ref": parts[0], "weight_g": int(parts[1]), "postcode": parts[2]}
'''
PARCELS_V2 = PARCELS_V1.replace('''    parts = line.strip().split(",")''', '''    parts = [p.strip() for p in line.strip().split(",")]''')

LABELS_V1 = '''"""Label sheets."""


def paginate(items, size):
    """Split items into consecutive lists of at most size items, in order, with no empty page. size must be at least 1."""
    if size < 1:
        raise ValueError("a page needs at least one row")
    return [list(items[i:i + size]) for i in range(0, len(items), size)]
'''
LABELS_V2 = LABELS_V1 + '''

def render_label(row):
    """One line of a label: the reference padded to 10 characters and the zone."""
    return "%-10s zone %d" % (row["ref"], row["zone"])
'''
LABELS_V3 = LABELS_V2.replace("%-10s", "%-12s").replace("padded to 10", "padded to 12")

LEDGER = '''"""Totals."""


def total_cents(amounts):
    """The sum of whole-cent amounts. A float is refused with TypeError: floats hide rounding errors."""
    total = 0
    for a in amounts:
        if not isinstance(a, int) or isinstance(a, bool):
            raise TypeError("an amount is a whole number of cents: %r" % (a,))
        total += a
    return total
'''

T_WEIGHTS = '''import unittest

from parcelkit.weights import block_count, to_grams


class TestWeights(unittest.TestCase):
    def test_to_grams_rounds(self):
        self.assertEqual(to_grams(1.2504), 1250)
        self.assertEqual(to_grams(0), 0)

    def test_block_count_counts_started_blocks(self):
        self.assertEqual(block_count(0), 0)
        self.assertEqual(block_count(1), 1)
        self.assertEqual(block_count(100), 1)
        self.assertEqual(block_count(101), 2)
'''
T_ZONES = '''import unittest

from parcelkit.zones import zone_for


class TestZones(unittest.TestCase):
    def test_zones_by_first_digit(self):
        self.assertEqual([zone_for(p) for p in ("01234", "29999", "30000", "55555", "60000", "88888", "90000")],
                         [1, 1, 2, 2, 3, 3, 4])

    def test_a_bad_postcode_is_refused(self):
        for bad in ("1234", "123456", "12a45", ""):
            with self.assertRaises(ValueError):
                zone_for(bad)
'''
T_RATES = '''import unittest

from parcelkit.rates import rate_cents


class TestRates(unittest.TestCase):
    def test_base_and_blocks(self):
        self.assertEqual(rate_cents(0, 1), 450)
        self.assertEqual(rate_cents(100, 1), 485)
        self.assertEqual(rate_cents(250, 3), 810 + 3 * 62)

    def test_a_negative_net_weight_is_refused(self):
        with self.assertRaises(ValueError):
            rate_cents(-1, 2)
'''
T_PARCELS = '''import unittest

from parcelkit.parcels import parse_line


class TestParcels(unittest.TestCase):
    def test_a_line(self):
        self.assertEqual(parse_line("A1,500,12345\\n"), {"ref": "A1", "weight_g": 500, "postcode": "12345"})
        self.assertEqual(parse_line("A1 , 500 , 12345 "), {"ref": "A1", "weight_g": 500, "postcode": "12345"})

    def test_a_bad_line_is_refused(self):
        for bad in ("", "A1,500", ",500,12345", "A1,heavy,12345"):
            with self.assertRaises(ValueError):
                parse_line(bad)
'''
T_LABELS = '''import unittest

from parcelkit.labels import paginate, render_label


class TestLabels(unittest.TestCase):
    def test_pages_keep_order_and_are_never_empty(self):
        self.assertEqual(paginate([1, 2, 3, 4, 5], 2), [[1, 2], [3, 4], [5]])
        self.assertEqual(paginate([], 3), [])
        with self.assertRaises(ValueError):
            paginate([1], 0)

    def test_a_label_line(self):
        self.assertEqual(render_label({"ref": "A1", "zone": 2}), "A1           zone 2")
'''
T_LEDGER = '''import unittest

from parcelkit.ledger import total_cents


class TestLedger(unittest.TestCase):
    def test_sum_and_refusal_of_floats(self):
        self.assertEqual(total_cents([450, 485]), 935)
        self.assertEqual(total_cents([]), 0)
        with self.assertRaises(TypeError):
            total_cents([1.5])
'''
EMPTY = ""


def readme(status):
    return ("# parcelkit\n\nShipping manifests for a small warehouse. Eight tickets:\n\n"
            "1. weights (parcelkit/weights.py)\n2. zones (parcelkit/zones.py)\n3. rates (parcelkit/rates.py)\n"
            "4. parcels (parcelkit/parcels.py)\n5. labels (parcelkit/labels.py)\n6. ledger (parcelkit/ledger.py)\n"
            "7. manifest (parcelkit/manifest.py)\n8. command line (parcelkit/cli.py)\n\n"
            "Run the tests with `python3 -m unittest discover -s tests -t .`.\n\n" + status)


MANIFEST = '''"""Shipping manifests (ticket 7)."""
from parcelkit.labels import paginate
from parcelkit.ledger import total_cents
from parcelkit.parcels import parse_line
from parcelkit.rates import rate_cents
from parcelkit.zones import zone_for

TAG = @@TAG@@
PAGE_ROWS = @@PAGE@@
TARE_G = @@TARE@@


def manifest_filename(date, seq):
    """TAG-YYYYMMDD-SEQ.mf, SEQ zero padded to 3 digits."""
    return "%s-%s-%03d.mf" % (TAG, date.strftime("%Y%m%d"), seq)


def build_manifest(lines, date, seq):
    """The manifest of a list of scanner lines: its file name, the rows in pages, the count and the total in cents."""
    rows = []
    for line in lines:
        p = parse_line(line)
        zone = zone_for(p["postcode"])
        net = max(0, p["weight_g"] - TARE_G)
        rows.append({"ref": p["ref"], "weight_g": p["weight_g"], "zone": zone, "cents": rate_cents(net, zone)})
    return {"filename": manifest_filename(date, seq), "pages": paginate(rows, PAGE_ROWS), "count": len(rows),
            "total_cents": total_cents([r["cents"] for r in rows])}
'''

MANIFEST_TEST = '''import datetime
import unittest

from parcelkit.manifest import build_manifest, manifest_filename


class TestManifest(unittest.TestCase):
    def test_the_file_name_has_the_agreed_shape(self):
        self.assertRegex(manifest_filename(datetime.date(2026, 11, 20), 7), r"^[a-z0-9]+-20261120-007\\.mf$")

    def test_rows_are_in_order_and_counted(self):
        m = build_manifest(["A1,900,12345", "B2,1500,90210"], datetime.date(2026, 11, 20), 1)
        self.assertEqual(m["count"], 2)
        self.assertEqual([r["ref"] for p in m["pages"] for r in p], ["A1", "B2"])
        self.assertEqual(m["total_cents"], sum(r["cents"] for p in m["pages"] for r in p))
'''

DUP_MANIFEST_EXTRA = '''

def paginate(items, size):
    """Pages of at most size items (my own copy)."""
    return [list(items[i:i + size]) for i in range(0, len(items), size)]
'''


def manifest(tag, page, tare):
    return MANIFEST.replace("@@TAG@@", repr(tag)).replace("@@PAGE@@", str(page)).replace("@@TARE@@", str(tare))
