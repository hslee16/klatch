You are agent b of three (a, b, c) working one after another on the `chainkit` repository (a small inventory reader,
standard library only; see README.md). The job has four steps: 1 the schema file, 2 the bin codes, 3 the loader, 4 the
report. Agent a does step 1 and starts step 2, then stops; agent c finishes step 2 and does step 4. Step 3 is yours.

Step 3: write `chainkit/loader.py` with `load(data_path=None, schema_path=None)`. It returns a list of dicts, one per record of
the data file (default `data/inventory.dat` in the repository), with the keys `sku`, `bin` and `note` as stripped strings and
`qty` as an int. Where each column sits comes from the schema file (default `chainkit/schema.json` next to the module), which
agent a is making with `tools/make_schema.py`: do not write the columns into the loader, and do not publish the loader
until you have run it against the real schema file and the real data. The loader has to work with any layout the schema
file describes.

Publish to main, and make sure main passes the full test run when you finish. Do not redo another agent's step.
