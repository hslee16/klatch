#!/usr/bin/env python3
"""Make chainkit/schema.json from data/ruler.txt: the column spans of the fixed-width inventory file.

Run from the repository root: python3 tools/make_schema.py
The ruler is one line, name=width pairs separated by commas, in column order.
"""
import json
import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def main():
    with open(os.path.join(ROOT, "data", "ruler.txt")) as f:
        pairs = [p.split("=") for p in f.read().strip().split(",")]
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
