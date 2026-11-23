You are agent c of three (a, b, c) working one after another on the `chainkit` repository (a small inventory reader,
standard library only; see README.md). The job has four steps: 1 the schema file, 2 the bin codes, 3 the loader, 4 the
report. Agent a does step 1 and starts step 2, then is reassigned and stops, and hands step 2 over to you. Agent b does step 3.
Steps 2 (the finishing) and 4 are yours.

Step 2: finish `chainkit/bins.py`. Agent a has written `parse_bin` and left `canonical_bin` unfinished. `canonical_bin(raw)`
returns the canonical code of a bin, `PREFIX-NNNNN`. What a knows about how bins are really named is not in the repository:
you learn it from a's handoff. Keep what a did, and do not redo it.

Step 4: write `chainkit/report.py` with `bin_totals(data_path=None)`. It returns a dict from canonical bin code to the total
`qty` of all records in the data file (default `data/inventory.dat`) with that canonical code. It needs the loader (step 3,
agent b: `chainkit.loader.load`) and `canonical_bin`; do not write your own loader, and wait for b's.

Publish to main, and make sure main passes the full test run when you finish.
