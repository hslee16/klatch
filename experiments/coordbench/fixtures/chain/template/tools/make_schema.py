#!/usr/bin/env python3
"""Make chainkit/schema.json from a ruler: the column spans of the fixed-width inventory file.

Run from the repository root: python3 tools/make_schema.py RULER
The ruler is one argument, name=width pairs separated by commas, in column order. It is not kept in the repository (it
comes from the vendor of the data file), so whoever makes the schema has to be given it.
"""
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: python3 tools/make_schema.py RULER   (name=width pairs, comma separated, in column order)")
    pairs = [p.split("=") for p in sys.argv[1].strip().split(",")]
    fields, pos = [], 0
    for name, width in pairs:
        fields.append({"name": name, "start": pos, "end": pos + int(width)})
        pos += int(width)
    out = os.path.join(ROOT, "chainkit", "schema.json")
    with open(out, "w") as f:
        json.dump({"fields": fields, "record_length": pos}, f, indent=1, sort_keys=True)
        f.write("\n")
    print("wrote %s (%d fields)" % (os.path.relpath(out, ROOT), len(fields)))


if __name__ == "__main__":
    main()
