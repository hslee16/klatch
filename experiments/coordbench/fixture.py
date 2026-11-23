"""Fixture interface (SPEC section 3). A fixture is a directory fixtures/<name>/ holding:

  fixture.json   name, description, topology, agents [{name, task: FILE}], test_command, checker (the name of a
                 directory under checks/), est_cost_usd_haiku (optional), fake (a fake-agent script, optional)
  setup.sh       setup.sh DEST SEED writes the initial tree into the empty directory DEST. The harness does the git
                 init, the first commit and the push to the per-run bare origin.
  task-*.md      the text each agent gets as its job
  private/<agent>/  optional ("private"): files only that agent's clone has (not in origin, excluded from git)
  seed.json      optional ("seed_history"): an earlier exchange present from the start: "room" (messages posted into the
                 room before any agent starts, K arms) and "board" (lines appended to TASKS.md, B1)

Optional keys of fixture.json (#1374, scenario S9):
  prebuilt_history  true when setup.sh builds a git repository itself (a scripted history): the harness then makes no first
                    commit of its own and pushes DEST's main to the per-run origin as it is. setup.sh learns the arm from the
                    environment variable COORDBENCH_ARM, so an arm can have extra files (a task board) inside that history.
  room_history      a JSON file {"steps": [{"as", "argv", "key"?}]}: klatch commands run as earlier members into the run's
                    temporary store before any agent starts (K arms only), see history.post_room
  variants          {"name": {overrides}}: a variant replaces top-level keys of the spec. The variant "resume" carries
                    {"resume": {"agent", "after_turns", "note": FILE}}: the agent's session is cut after that many turns and a
                    second session with the same name and a short note starts (run.py). A variant other than "join" renames
                    the fixture "<name>-<variant>" in records, so results of different variants are never pooled.

The hidden checker is NOT in the fixture: checks/<checker>/check.sh CHECKOUT prints {"checks": [{"id", "pass",
"unit"?}]} for a clone of the final origin. It is never copied into a checkout and never named in a prompt.
"""
import hashlib
import json
import os
import subprocess

HERE = os.path.dirname(os.path.abspath(__file__))


class FixtureError(Exception):
    pass


class Fixture:
    def __init__(self, name, fixtures_dir=None, checks_dir=None, variant=None):
        self.name = name
        self.variant = variant
        self.dir = os.path.join(fixtures_dir or os.path.join(HERE, "fixtures"), name)
        self.checks_dir = checks_dir or os.path.join(HERE, "checks")
        try:
            with open(os.path.join(self.dir, "fixture.json")) as f:
                self.spec = json.load(f)
        except (OSError, ValueError) as e:
            raise FixtureError("cannot load fixture %s: %s" % (name, e))
        if variant:
            variants = self.spec.get("variants") or {}
            if variant not in variants:
                raise FixtureError("fixture %s has no variant %r (it has: %s)" % (name, variant, ", ".join(sorted(variants)) or "none"))
            self.spec = dict(self.spec, **variants[variant])
            if variant != "join":
                self.name = "%s-%s" % (name, variant)
        self.agents = []
        for a in self.spec["agents"]:
            with open(os.path.join(self.dir, a["task"])) as f:
                self.agents.append({"name": a["name"], "task": f.read().strip()})
        self.test_command = self.spec.get("test_command", "python3 -m unittest")
        self.topology = self.spec.get("topology", "T-clone")
        self.checker = os.path.join(self.checks_dir, self.spec["checker"], "check.sh")
        self.fake_script = os.path.join(self.dir, self.spec["fake"]) if self.spec.get("fake") else None
        self.private_dir = os.path.join(self.dir, self.spec["private"]) if self.spec.get("private") else None
        self.seed_history = None
        if self.spec.get("seed_history"):
            try:
                with open(os.path.join(self.dir, self.spec["seed_history"])) as f:
                    self.seed_history = json.load(f)
            except (OSError, ValueError) as e:
                raise FixtureError("cannot load the seed history of fixture %s: %s" % (name, e))

    def private_files(self, agent):
        """[(path relative to the clone, absolute source)] of the files only this agent has (fixture key "private":
        private/<agent>/...). The harness copies them into that agent's clone, untracked and excluded from git."""
        if not self.private_dir:
            return []
        root = os.path.join(self.private_dir, agent)
        out = []
        for d, _, names in os.walk(root):
            for n in sorted(names):
                p = os.path.join(d, n)
                out.append((os.path.relpath(p, root), p))
        return sorted(out)
        self.prebuilt_history = bool(self.spec.get("prebuilt_history"))
        self.room_script = None
        if self.spec.get("room_history"):
            try:
                with open(os.path.join(self.dir, self.spec["room_history"])) as f:
                    self.room_script = json.load(f)["steps"]
            except (OSError, ValueError, KeyError) as e:
                raise FixtureError("cannot load the room history of fixture %s: %s" % (name, e))
        self.resume = None
        if self.spec.get("resume"):
            self.resume = dict(self.spec["resume"])
            try:
                with open(os.path.join(self.dir, self.resume["note"])) as f:
                    self.resume["note"] = f.read().strip()
            except (OSError, KeyError) as e:
                raise FixtureError("cannot load the resume note of fixture %s: %s" % (name, e))

    @property
    def names(self):
        return [a["name"] for a in self.agents]

    def sha256(self):
        """A hash of the fixture directory (relative paths and bytes, sorted), recorded in every result."""
        h = hashlib.sha256()
        for d, dirs, files in os.walk(self.dir):
            dirs.sort()
            for n in sorted(files):
                p = os.path.join(d, n)
                h.update(os.path.relpath(p, self.dir).encode() + b"\0")
                with open(p, "rb") as f:
                    h.update(f.read() + b"\0")
        return h.hexdigest()

    def materialise(self, dest, seed, env, arm=None):
        os.makedirs(dest, exist_ok=True)
        if arm:
            env = dict(env, COORDBENCH_ARM=arm)
        p = subprocess.run(["bash", os.path.join(self.dir, "setup.sh"), dest, str(seed)], stdout=subprocess.PIPE,
                           stderr=subprocess.STDOUT, text=True, env=env)
        if p.returncode != 0:
            raise FixtureError("fixture setup.sh exited %d: %s" % (p.returncode, p.stdout.strip()[-300:]))

    def run_checker(self, checkout, env, timeout=120):
        """Run the hidden checker on a checkout. Returns (checks, raw_stdout, error). error is None when the checker
        ran and printed valid JSON; then checks is its list. Otherwise checks is [] and error says why."""
        if not os.path.exists(self.checker):
            return [], "", "checker %s does not exist" % self.checker
        try:
            p = subprocess.run(["bash", self.checker, checkout], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               text=True, env=env, timeout=timeout)
        except subprocess.TimeoutExpired:
            return [], "", "checker timed out"
        if p.returncode != 0:
            return [], p.stdout, "checker exited %d" % p.returncode
        try:
            checks = json.loads(p.stdout.strip().splitlines()[-1])["checks"]
            assert isinstance(checks, list) and all(isinstance(c, dict) and "id" in c and "pass" in c for c in checks)
        except (ValueError, KeyError, IndexError, AssertionError):
            return [], p.stdout, "checker output is not the expected JSON"
        return [{k: v for k, v in c.items()} for c in checks], p.stdout, None
