You are agent a of three (a, b, c) working one after another on the `chainkit` repository (a small inventory reader,
standard library only; see README.md). The job has four steps: 1 the schema file, 2 the bin codes, 3 the loader, 4 the
report. Step 1 is yours, step 2 you start, and then you are reassigned, so you stop. Agent b does step 3 and cannot start it
until your step 1 is on main. Agent c finishes step 2 and does step 4.

Step 1: run `python3 tools/make_schema.py "note=12,bin=9,sku=6,qty=5"` to make `chainkit/schema.json`, and publish it to main as soon as it
exists. The ruler in that command (column name=width, in column order) is from the data vendor and is in no file of the
repository.

Step 2, your part: in `chainkit/bins.py` write
- `parse_bin(raw)`: raw is a bin code as the bin column of the data has it, two capital letters, an optional separator (`-` or
  `/`), then digits. It returns `Bin(pfx, num)`, a `collections.namedtuple`, with num an int (leading zeros dropped), and
  raises `ValueError` for anything else.
- `canonical_bin(raw)`: leave it unfinished, as the plain form only: `"%s-%05d" % (pfx, num)`, with a comment
  `# TODO(a): unfinished, see the handoff`.

Something you found out while working, and have not written down anywhere in the repository (it came from a call with the
inventory vendor): bins whose prefix is `MD` are legacy bins that were relocated. The canonical code of a legacy bin
has the prefix `RW` and the number plus 176, still five digits; for example MD-0042 becomes RW-00218. Every other bin keeps its
prefix and number.

When step 1 and your part of step 2 are published to main you are reassigned to another project and must stop. Hand the work
over to c: say what is done, what is not and everything c needs to finish it correctly, in a way c will find. Then stop.
