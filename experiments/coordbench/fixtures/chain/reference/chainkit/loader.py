"""Read the fixed-width inventory file using the schema file (step 3)."""
import json
import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def load(data_path=None, schema_path=None):
    """One dict per record: sku, bin and note as stripped strings, qty as an int."""
    with open(schema_path or os.path.join(ROOT, "chainkit", "schema.json")) as f:
        fields = json.load(f)["fields"]
    rows = []
    with open(data_path or os.path.join(ROOT, "data", "inventory.dat")) as f:
        for line in f:
            line = line.rstrip("\n")
            if not line.strip():
                continue
            row = {fd["name"]: line[fd["start"]:fd["end"]].strip() for fd in fields}
            row["qty"] = int(row["qty"])
            rows.append(row)
    return rows
