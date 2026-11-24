# chainkit

A small inventory reader, standard library only. Run the tests from the repository root with
`python3 -m unittest discover -s tests -t .`.

- `data/inventory.dat`: the stock records, one per line, fixed-width columns (the order and width of the columns come from the
  data vendor, not from this repository).
- `tools/make_schema.py RULER`: makes `chainkit/schema.json` (the column spans) from the ruler it is given.
- `chainkit/bins.py`: bin codes (step 2).
- `chainkit/loader.py`: reads the data file using the schema (step 3).
- `chainkit/report.py`: stock totals by canonical bin code (step 4).

Three agents work on it, one step after another (see your task text). Publish to `main`.
