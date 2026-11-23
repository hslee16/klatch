"""Stock totals by canonical bin code (step 4)."""
from chainkit import bins, loader


def bin_totals(data_path=None):
    """{canonical bin code: total qty}."""
    totals = {}
    for row in loader.load(data_path):
        code = bins.canonical_bin(row["bin"])
        totals[code] = totals.get(code, 0) + row["qty"]
    return totals
