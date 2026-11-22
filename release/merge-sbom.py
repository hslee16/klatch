#!/usr/bin/env python3
"""Merge the CycloneDX 1.5 SBOMs of klatch and klatch-server into the one SBOM an archive carries (#1236).

    merge-sbom.py klatch.cdx.json klatch-server.cdx.json merged.cdx.json

The merged document keeps klatch's metadata (the archive is named for klatch), lists klatch-server's own component as a
component, and holds every component and dependency of both once (by bom-ref). Standard library only.
"""
import json
import sys


def load(path):
    with open(path) as f:
        bom = json.load(f)
    if bom.get("bomFormat") != "CycloneDX" or bom.get("specVersion") != "1.5":
        sys.exit(f"merge-sbom: {path} is not a CycloneDX 1.5 SBOM")
    return bom


def key_of(component):
    return component.get("bom-ref") or (component.get("name"), component.get("version"))


def main(argv):
    if len(argv) != 4:
        sys.exit(__doc__)
    base, server = load(argv[1]), load(argv[2])
    merged = dict(base)

    components = list(base.get("components", []))
    seen = {key_of(c) for c in components}
    extra = list(server.get("components", []))
    main_component = server.get("metadata", {}).get("component")
    if main_component:
        extra.insert(0, main_component)
    for c in extra:
        if key_of(c) not in seen:
            seen.add(key_of(c))
            components.append(c)
    merged["components"] = components

    deps = {d["ref"]: d for d in base.get("dependencies", [])}
    for d in server.get("dependencies", []):
        if d["ref"] in deps:
            have = deps[d["ref"]]
            have["dependsOn"] = sorted(set(have.get("dependsOn", [])) | set(d.get("dependsOn", [])))
        else:
            deps[d["ref"]] = d
    merged["dependencies"] = list(deps.values())

    with open(argv[3], "w") as f:
        json.dump(merged, f, indent=2)
        f.write("\n")


if __name__ == "__main__":
    main(sys.argv)
