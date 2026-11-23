#!/usr/bin/env python3
"""Compare a results directory with a previous release's: compare.py NEW_RESULTS_DIR OLD_RESULTS_DIR (#1364).

A thin wrapper over `report.py NEW --baseline OLD --comparison-only`: it prints the comparison, with every regression
flagged (a median worse than the previous median by more than the previous interquartile range; confirmed only when
the Holm-corrected test agrees and N is at least 5). Other options are report.py's (--primary, --seed, --resamples,
--json).
"""
import sys

import report


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    if len(argv) < 2 or argv[0].startswith("-") or argv[1].startswith("-"):
        print("usage: compare.py NEW_RESULTS_DIR OLD_RESULTS_DIR [report.py options]", file=sys.stderr)
        return 2
    new, old, rest = argv[0], argv[1], argv[2:]
    return report.main([new, "--baseline", old, "--comparison-only"] + rest)


if __name__ == "__main__":
    sys.exit(main())
