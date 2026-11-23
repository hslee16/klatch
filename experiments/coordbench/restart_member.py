#!/usr/bin/env python3
"""Ask the harness to start a replacement for a worker that stopped without finishing (#1421).

    restart_member.py NAME [--wait-s N]

The supervisor of the arms K3 and B1-roles runs this (it is what {{RESTART_COMMAND}} in its prompt names). It does not start
anything itself: it writes the request file $COORDBENCH_RESTART_DIR/NAME.request and waits for the harness to answer in
NAME.result ("started: REPLACEMENT" or "refused: REASON"). The harness starts the replacement as a new member with a clone of
its own, charged to the roles, and refuses when the worker is still running, is not a worker, or the restart limit is spent.
Exit codes: 0 started, 1 refused or no answer, 2 usage (an unknown name, or none).
"""
import argparse
import os
import re
import sys
import time


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("name", nargs="?")
    p.add_argument("--wait-s", type=float, default=20.0)
    a = p.parse_args(argv)
    d = os.environ.get("COORDBENCH_RESTART_DIR")
    if not d or not os.path.isdir(d):
        print("restart_member: no restart channel in this run")
        return 1
    try:
        with open(os.path.join(d, "workers")) as f:
            workers = [l.strip() for l in f if l.strip()]
    except OSError:
        workers = []
    if not a.name or not re.fullmatch(r"[A-Za-z0-9._-]+", a.name) or a.name not in workers:
        print("usage: restart_member.py NAME; NAME is a worker: %s" % ", ".join(workers))
        return 2
    tmp = os.path.join(d, a.name + ".request.tmp")
    with open(tmp, "w") as f:
        f.write("restart\n")
    os.replace(tmp, os.path.join(d, a.name + ".request"))
    result = os.path.join(d, a.name + ".result")
    deadline = time.monotonic() + a.wait_s
    while time.monotonic() < deadline:
        if os.path.exists(result):
            with open(result) as f:
                text = f.read().strip()
            os.remove(result)
            print(text)
            return 0 if text.startswith("started") else 1
        time.sleep(0.05)
    print("no answer in %d s; the request is still pending" % a.wait_s)
    return 1


if __name__ == "__main__":
    sys.exit(main())
