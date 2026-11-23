#!/usr/bin/env bash
# Fixture interface: setup.sh DEST SEED. Writes the initial tree of the repository into the empty directory DEST. The
# harness does the git init, the first commit and the push to the per-run bare origin; setup.sh makes no git calls and
# touches nothing outside DEST. SEED is unused by this trivial fixture.
set -euo pipefail
dest="${1:?usage: setup.sh DEST SEED}"
mkdir -p "$dest/tests"
cat > "$dest/alpha.py" <<'PY'
def alpha():
    raise NotImplementedError
PY
cat > "$dest/beta.py" <<'PY'
def beta():
    raise NotImplementedError
PY
: > "$dest/tests/__init__.py"
cat > "$dest/tests/test_alpha.py" <<'PY'
import unittest

import alpha


class TestAlpha(unittest.TestCase):
    def test_returns_a_string(self):
        self.assertIsInstance(alpha.alpha(), str)
PY
cat > "$dest/tests/test_beta.py" <<'PY'
import unittest

import beta


class TestBeta(unittest.TestCase):
    def test_returns_a_string(self):
        self.assertIsInstance(beta.beta(), str)
PY
