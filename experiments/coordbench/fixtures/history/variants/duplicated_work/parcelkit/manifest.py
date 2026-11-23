"""Shipping manifests (ticket 7)."""
from parcelkit.ledger import total_cents
from parcelkit.parcels import parse_line
from parcelkit.rates import rate_cents

TAG = 'hb2g'
PAGE_ROWS = 85
TARE_G = 419


def paginate(items, size):
    """Pages of at most size items (my own copy)."""
    return [list(items[i:i + size]) for i in range(0, len(items), size)]


def zone_for(postcode):
    """My own zone rule."""
    first = int(postcode[0])
    return 1 if first <= 2 else 2 if first <= 5 else 3 if first <= 8 else 4


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
