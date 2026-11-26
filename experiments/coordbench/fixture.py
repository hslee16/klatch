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
  injection         (#1375, S10) a JSON file of injection forms (injections.py): a scripted colleague delivers them in the
                    room (K arms) or on TASKS.md (B1); the harness writes the run's plan to the log directory and passes it to the
                    hidden checker (COORDBENCH_INJECTION_PLAN). injection_mode: "all" (every form) or "one" (the seed picks)
  topology          "T-clone" (the default: one clone per agent and a bare origin) or "T-shared" (#1427: one working tree, all agents
                    start in it; run.py makes it a clone of the bare origin, gives each agent its own identity and its own
                    HOME and TMPDIR outside it, and scores the tree committed at the end; see shared.py). A fixture takes it
                    from a variant ("shared"), so a T-shared result is the separate scenario "<name>-shared".
  variants          {"name": {overrides}}: a variant replaces top-level keys of the spec. The variant "resume" carries
                    {"resume": {"agent", "after_turns", "note": FILE}}: the agent's session is cut after that many turns and a
                    second session with the same name and a short note starts (run.py). A variant other than "join" renames
                    the fixture "<name>-<variant>" in records, so results of different variants are never pooled.

generator_sources  (#1516) the files of the fixture directory that make committed files or the repository setup.sh builds (gen.py,
                    gen_facts.py, the modules they import, build_history.py). They are not in the fixture hash: their output is
                    committed and proved by the generator's --check, or it is hashed (see fixhash.py, which says what the hash covers).

Generated fixtures (#1372, scenario S7): "generated": {"module": "gen.py", "modules": M, "seed": S} names a generator in the
fixture directory with the functions tasks(M, S) -> [{"name", "task"}] (the agents and their job text, instead of "agents"
and task files) and scenario_map(M, S) (the unit-to-path map the scenario file leaves empty). The harness gives setup.sh and the
checker COORDBENCH_GEN_MODULES and COORDBENCH_GEN_SEED. The seed belongs to the fixture, not to the run: every run of a size
gets the same repository. "require_variant": true refuses the fixture without a variant (a size), so the sizes are never
recorded under one name.

Seeded facts (#1531): "seeded_facts": {"default_seed": N, "outputs": [paths of the fixture directory that carry the hidden facts],
"checker_files": [files of the checker directory that carry them]} says that gen_facts.py draws the hidden facts from a seed:
generate(seed) -> {path relative to experiments/coordbench: text}. The committed files are the default seed's copy (the fake
driver tests use it, --committed-facts). A run writes the seed's files into a directory outside every agent's reach
(with_facts), takes the private files, the seed and room files, the fake script and the checker's facts from there, and the
checker finds its facts through COORDBENCH_FACTS. fixhash.py says what the hash covers for such a fixture.

The hidden checker is NOT in the fixture: checks/<checker>/check.sh CHECKOUT prints {"checks": [{"id", "pass",
"unit"?}]} for a clone of the final origin. It is never copied into a checkout and never named in a prompt.
"""
import copy
import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile

import fixhash

HERE = os.path.dirname(os.path.abspath(__file__))


class FixtureError(Exception):
    pass


def _parse_checks(stdout):
    """The checks list of the checker's last output line, or None when it is not the expected JSON."""
    try:
        checks = json.loads(stdout.strip().splitlines()[-1])["checks"]
        assert isinstance(checks, list) and all(isinstance(c, dict) and "id" in c and "pass" in c for c in checks)
    except (ValueError, KeyError, IndexError, AssertionError):
        return None
    return checks


class Fixture:
    def __init__(self, name, fixtures_dir=None, checks_dir=None, variant=None):
        self.name = name
        self.requested = name  # the name the caller gave, which the error messages use (self.name gains the variant)
        self.variant = variant
        self.dir = os.path.join(fixtures_dir or os.path.join(HERE, "fixtures"), name)
        self.checks_dir = checks_dir or os.path.join(HERE, "checks")
        try:
            with open(os.path.join(self.dir, "fixture.json")) as f:
                self.spec = json.load(f)
        except (OSError, ValueError) as e:
            raise FixtureError("cannot load fixture %s: %s" % (name, e))
        self._apply_variant(name, variant)
        self.generated = self.spec.get("generated")
        self._gen = None
        self._sha = None
        self.generator_sources = list(self.spec.get("generator_sources") or [])
        self.agents = self._load_agents(name)
        self.test_command = self.spec.get("test_command", "python3 -m unittest")
        self.topology = self.spec.get("topology", "T-clone")
        if self.topology == "T-shared" and self.spec.get("private"):
            raise FixtureError("fixture %s: a shared working tree (T-shared) cannot hold files only one agent has" % name)
        self.checker = os.path.join(self.checks_dir, self.spec["checker"], "check.sh")
        self.fake_script = os.path.join(self.dir, self.spec["fake"]) if self.spec.get("fake") else None
        self.private_dir = os.path.join(self.dir, self.spec["private"]) if self.spec.get("private") else None
        self.seed_history = self._load_json("seed_history", "the seed history", OSError, ValueError)
        self.prebuilt_history = bool(self.spec.get("prebuilt_history"))
        self.room_script = self._load_json("room_history", "the room history", OSError, ValueError, KeyError,
                                           pick="steps")
        self.injection_mode = self.spec.get("injection_mode", "all")
        self.injection = self._load_json("injection", "the injections", OSError, ValueError)
        self.resume = self._load_resume(name)
        self.seeded = self.spec.get("seeded_facts")
        self.facts_path = None  # the checker's facts of this run (with_facts), else the committed default-seed copy
        self.facts_root = None

    def _apply_variant(self, name, variant):
        variants = self.spec.get("variants") or {}
        if self.spec.get("require_variant") and not variant:
            raise FixtureError("fixture %s needs a variant (%s)" % (name, ", ".join(sorted(variants))))
        if variant:
            self._select_variant(name, variant, variants)

    def _select_variant(self, name, variant, variants):
        if variant not in variants:
            raise FixtureError("fixture %s has no variant %r (it has: %s)" % (name, variant, ", ".join(sorted(variants)) or "none"))
        self.spec = dict(self.spec, **variants[variant])
        if variant != "join":
            self.name = "%s-%s" % (name, variant)

    def _load_agents(self, name):
        """[{"name", "task"}]: generated by the fixture's module, or read from the task files its spec names."""
        if self.generated:
            try:
                return [{"name": t["name"], "task": t["task"].strip()} for t in self.gen().tasks(
                    self.generated["modules"], self.generated["seed"])]
            except (OSError, KeyError, ValueError, AttributeError) as e:
                raise FixtureError("cannot generate fixture %s: %s" % (name, e))
        agents = []
        for a in self.spec["agents"]:
            with open(os.path.join(self.dir, a["task"])) as f:
                agents.append({"name": a["name"], "task": f.read().strip()})
        return agents

    def _load_json(self, key, what, *errors, pick=None):
        """The JSON file the spec names under key (the value under pick when given), or None when the spec has no such key."""
        if not self.spec.get(key):
            return None
        try:
            with open(os.path.join(self.dir, self.spec[key])) as f:
                obj = json.load(f)
            return obj[pick] if pick else obj
        except errors as e:
            raise FixtureError("cannot load %s of fixture %s: %s" % (what, self.requested, e))

    def _load_resume(self, name):
        if not self.spec.get("resume"):
            return None
        resume = dict(self.spec["resume"])
        try:
            with open(os.path.join(self.dir, resume["note"])) as f:
                resume["note"] = f.read().strip()
        except (OSError, KeyError) as e:
            raise FixtureError("cannot load the resume note of fixture %s: %s" % (name, e))
        return resume

    def gen(self):
        """The fixture's generator module, loaded from its file (None for a fixture that is not generated)."""
        if self._gen is None and self.generated:
            path = os.path.join(self.dir, self.generated["module"])
            spec = importlib.util.spec_from_file_location("fixture_gen_%s" % self.spec["name"], path)
            self._gen = importlib.util.module_from_spec(spec)
            previous, sys.dont_write_bytecode = sys.dont_write_bytecode, True  # no __pycache__ in the fixture (#1510)
            try:
                spec.loader.exec_module(self._gen)
            finally:
                sys.dont_write_bytecode = previous
        return self._gen

    def gen_env(self):
        """The environment setup.sh and the checker read for a generated fixture ({} otherwise)."""
        if not self.generated:
            return {}
        return {"COORDBENCH_GEN_MODULES": str(self.generated["modules"]), "COORDBENCH_GEN_SEED": str(self.generated["seed"])}

    def scenario_map(self):
        """The generated unit, agent and shared-file map to merge into the scenario file ({} when not generated)."""
        return self.gen().scenario_map(self.generated["modules"], self.generated["seed"]) if self.generated else {}

    @property
    def default_seed(self):
        """The seed of the committed facts files (None for a fixture whose facts are not seeded)."""
        return self.seeded["default_seed"] if self.seeded else None

    def facts_seed(self, run_seed, committed=False):
        """The seed the facts of a run are drawn from: the run's seed, or the committed default with --committed-facts
        (None when the fixture's facts are not seeded)."""
        return None if not self.seeded else (self.default_seed if committed else run_seed)

    def facts_files(self, seed):
        """{path relative to experiments/coordbench: text}: every file the generator makes for the seed (drawn in process)."""
        path = os.path.join(self.dir, "gen_facts.py")
        spec = importlib.util.spec_from_file_location("fixture_facts_%s" % self.spec["name"], path)
        mod = importlib.util.module_from_spec(spec)
        previous, sys.dont_write_bytecode = sys.dont_write_bytecode, True  # no __pycache__ in the fixture (#1510)
        try:
            spec.loader.exec_module(mod)
            return mod.generate(seed)
        finally:
            sys.dont_write_bytecode = previous

    def facts_sha256(self, seed):
        """What the record says the run's facts were: the sha256 of every generated file (sorted paths and bytes), or None."""
        if not self.seeded:
            return None
        h = hashlib.sha256()
        for rel, text in sorted(self.facts_files(seed).items()):
            h.update(rel.encode() + b"\0" + text.encode() + b"\0")
        return h.hexdigest()

    def with_facts(self, seed, root):
        """A copy of this fixture whose private files, seed and room files, fake script and checker facts are the seed's, written
        under root (made by the harness, never inside an agent's reach). The facts file of the checker is root/checks/<checker>/."""
        fx = copy.copy(self)
        name = os.path.basename(self.dir)
        ref = os.path.join(self.dir, "reference")
        if os.path.isdir(ref):  # the parts of reference/ no seed changes (recovery: bits, pairs, wrap) come from the fixture
            shutil.copytree(ref, os.path.join(root, "fixtures", name, "reference"), dirs_exist_ok=True,
                            ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
        for rel, text in self.facts_files(seed).items():
            path = os.path.join(root, rel)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w") as f:
                f.write(text)
        own = os.path.join(root, "fixtures", name)
        fx.facts_root = root
        fx.facts_path = os.path.join(root, "checks", self.spec["checker"], "facts.json")
        if self.private_dir:
            fx.private_dir = os.path.join(own, self.spec["private"])
        if self.fake_script:
            fx.fake_script = os.path.join(own, os.path.basename(self.fake_script))
            if not os.path.exists(fx.fake_script):  # a script the seed does not change still finds reference/ next to it
                os.makedirs(own, exist_ok=True)
                shutil.copyfile(self.fake_script, fx.fake_script)
        fx.seed_history = self._reload_json(own, "seed_history", "seed history") if self.seed_history else None
        fx.room_script = self._reload_json(own, "room_history", "room history", pick="steps") if self.room_script else None
        return fx

    def _reload_json(self, base, key, what, pick=None):
        try:
            with open(os.path.join(base, self.spec[key])) as f:
                obj = json.load(f)
            return obj[pick] if pick else obj
        except (OSError, ValueError, KeyError):
            return getattr(self, {"seed_history": "seed_history", "room_history": "room_script"}[key])  # not generated: the committed one

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

    @property
    def names(self):
        return [a["name"] for a in self.agents]

    def sha256(self):
        """The fixture hash, version fixhash.FIXTURE_HASH_VERSION (what it covers is in fixhash.py), recorded in every result.
        Computed once per object, so every record of a suite has the same value."""
        if self._sha is None:
            h = hashlib.sha256()
            if self.generated:  # the generator's parameters are part of what ran
                h.update(json.dumps(self.generated, sort_keys=True).encode() + b"\0")
            omit = self.seeded["outputs"] if self.seeded else self.generator_sources  # #1531: see fixhash.py, "seeded"
            fixhash.tree_update(h, self.dir, omit=omit)
            self._output_update(h)
            self._sha = h.hexdigest()
        return self._sha

    def checker_sha256(self):
        """The hash of the hidden checker directory checks/<checker>/ (None when it does not exist)."""
        d = os.path.dirname(self.checker)
        omit = self.seeded.get("checker_files", []) if self.seeded else []
        return fixhash.tree_sha256(d, omit=omit) if os.path.isdir(d) else None

    def _output_update(self, h):
        """What the generator sources make and the directory does not hold (fixhash.py): a generated fixture's template,
        reference, tasks and map; a prebuilt history's commit id in each board kind of arm."""
        if self.generated:
            with tempfile.TemporaryDirectory(prefix="cb-fixhash-") as tmp:
                self.materialise(os.path.join(tmp, "template"), 0, self._hash_env())
                self._run_generator(["reference", os.path.join(tmp, "reference")])
                for part in ("template", "reference"):
                    h.update(b"generated:" + part.encode() + b"\0")
                    fixhash.tree_update(h, os.path.join(tmp, part))
            h.update(json.dumps([self.agents, self.scenario_map()], sort_keys=True).encode() + b"\0")
        if self.prebuilt_history and not self.seeded:  # a seeded history's commit ids differ per seed; its sources are hashed
            for arm in ("B0", "B1"):
                h.update(b"history:" + arm.encode() + b"\0" + self._history_head(arm).encode() + b"\0")

    def _hash_env(self):
        return {k: os.environ[k] for k in ("PATH", "LANG", "LC_ALL") if k in os.environ}

    def _run_generator(self, args):
        cmd = [sys.executable, "-B", "-I", os.path.join(self.dir, self.generated["module"])] + args + [
            "--modules", str(self.generated["modules"]), "--seed", str(self.generated["seed"])]
        p = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env=self._hash_env())
        if p.returncode != 0:
            raise FixtureError("fixture %s: the generator failed: %s" % (self.requested, p.stdout.strip()[-300:]))

    def _history_head(self, arm):
        """The commit id of main in the repository setup.sh builds for arm (ids are the same in every run)."""
        with tempfile.TemporaryDirectory(prefix="cb-fixhash-") as tmp:
            dest = os.path.join(tmp, "repo")
            self.materialise(dest, 0, self._hash_env(), arm=arm)
            p = subprocess.run(["git", "-C", dest, "rev-parse", "main"], stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                               text=True, env=dict(self._hash_env(), GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1"))
            if p.returncode != 0:
                raise FixtureError("fixture %s: no history built for arm %s: %s" % (self.requested, arm, p.stdout.strip()[-200:]))
            return p.stdout.strip()

    def materialise(self, dest, seed, env, arm=None):
        os.makedirs(dest, exist_ok=True)
        env = dict(env, **self.gen_env())
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
        env = dict(env, **self.gen_env())
        if self.facts_path:  # #1531: this run's facts, outside the agents' reach; else the checker reads its committed copy
            env["COORDBENCH_FACTS"] = self.facts_path
        try:
            p = subprocess.run(["bash", self.checker, checkout], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               text=True, env=env, timeout=timeout)
        except subprocess.TimeoutExpired:
            return [], "", "checker timed out"
        if p.returncode != 0:
            return [], p.stdout, "checker exited %d" % p.returncode
        checks = _parse_checks(p.stdout)
        if checks is None:
            return [], p.stdout, "checker output is not the expected JSON"
        return [{k: v for k, v in c.items()} for c in checks], p.stdout, None
