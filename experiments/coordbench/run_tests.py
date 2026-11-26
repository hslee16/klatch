#!/usr/bin/env python3
"""Run the coordbench tests, one process per test module, a few at a time (#1471).

    python3 -I experiments/coordbench/run_tests.py          the fast subset (what the local gate runs)
    python3 -I experiments/coordbench/run_tests.py --all    every module (about ten minutes one after another)

The fast subset leaves out the modules in SLOW (each took 20 seconds or more on the machine that measured them); the full
command is `python3 -I -m unittest discover -s experiments/coordbench/tests` or --all here. The K-arm tests need KLATCH_BIN
(a built klatch); without it they skip with a message. Nothing here uses the network, a model or a credential: every test
works in temp directories with the fake agent."""
import argparse
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
TESTS = os.path.join(HERE, "tests")
SLOW = frozenset("""test_roles test_s7_scenario test_s9_scenario test_run_fake test_s6_scenario test_isolation_followup
test_s4_scenario test_t_shared test_scale_checker test_s3_scenario""".split())


def modules():
    return sorted(f[:-3] for f in os.listdir(TESTS) if f.startswith("test_") and f.endswith(".py"))


def selected(everything):
    return [m for m in modules() if everything or m not in SLOW]


def run_one(name):
    p = subprocess.Popen([sys.executable, "-I", "-m", "unittest", "discover", "-s", TESTS, "-p", name + ".py"],
                         stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    return name, p


def main(argv=None):
    ap = argparse.ArgumentParser(description="run the coordbench tests")
    ap.add_argument("--all", action="store_true", help="every module, not only the fast subset")
    ap.add_argument("--jobs", type=int, default=8)
    a = ap.parse_args(argv)
    todo, running, failed, started = selected(a.all), [], [], time.time()
    total = len(todo)
    while todo or running:
        while todo and len(running) < a.jobs:
            running.append(run_one(todo.pop(0)))
        name, p = running.pop(0)
        out = p.communicate()[0]
        if p.returncode != 0:
            failed.append(name)
            print("FAILED %s\n%s" % (name, "\n".join(out.splitlines()[-120:])))
    print("coordbench: %d of %d modules (%s), %d failed, %ds" % (total - len(failed), total, "all" if a.all else "fast subset",
                                                            len(failed), time.time() - started))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
