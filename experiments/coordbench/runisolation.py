"""The isolation part of one run (#1519), a mixin of runexec.RunExecution: the per-agent directories, who may name what,
hiding the scratch and log directories while agents run, and the post-run scan of every stream (SPEC 11).

Layout of the run scratch directory (mode 0300 while agents run, see isolation.py):

    <scratch>/<token>/clone    an agent's clone (T-clone; in T-shared the one shared tree is <scratch>/shared)
    <scratch>/<token>/home     its HOME;  tmp: its TMPDIR;  claude-config: its CLAUDE_CONFIG_DIR (not in B1 and B1-roles)
    <scratch>/agents/<token>   the ONE CLAUDE_CONFIG_DIR of every agent of a B1 or B1-roles run (#1520, see shared_config)
    <scratch>/origin.git, klatch-home, klatch-keys, bin (the restart script), restart, pwned (S10)
"""
import copy
import os
import shutil
import stat

import isolation
import messaging
from runenv import EXCLUDE, append_exclude, sha256_file

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.dirname(os.path.dirname(HERE))
STREAM_SUFFIX = ".stream.jsonl"


def _digest(path):
    """sha256 of a regular file (a socket or a vanished file has none: null)."""
    try:
        return sha256_file(path) if stat.S_ISREG(os.lstat(path).st_mode) else None
    except OSError:
        return None


class IsolationSupport:
    def init_isolation(self):
        self.agent_dirs = {}
        self.log_names = {}  # (session key, kind) -> file name in the log directory while the run is on
        self.log_public = {}  # file name -> public name it takes when the run ends
        self.iso_mode = getattr(self.a, "isolation", "sandbox")
        self.iso_result = None
        self.shared_cfg = None
        self.facts_dir = None

    def prepare_facts(self):
        """#1531: a fixture with seeded hidden facts gets this run's facts (drawn from the run's seed) written under a directory of the
        scratch with a random name, which is hidden from listing while the agents run (hide_run), guarded like the rest of the
        scratch (guard_dirs, the sandbox and the scan), and never inside an agent's clone, home, tmp or the log directory. The
        private files, the seed and room files and the fake script come from it; the checker's facts file is passed through the
        environment (Fixture.run_checker). Nothing is written for a fixture whose facts are not seeded."""
        self.setup_seed = self.seed
        if not self.fx.seeded:
            return
        self.facts_dir = os.path.join(self.scratch, "facts-" + isolation.token())
        os.makedirs(self.facts_dir, mode=0o700)
        seed = self.fx.facts_seed(self.seed, getattr(self.a, "committed_facts", False))
        self.setup_seed = seed
        self.fx = self.fx.with_facts(seed, self.facts_dir)
        if self.driver.name == "fake" and not getattr(self.a, "fake_script", None) and self.fx.fake_script:
            self.driver = copy.copy(self.driver)
            self.driver.script_path = os.path.abspath(self.fx.fake_script)

    # layout --------------------------------------------------------------------------------------------------

    def agent_dir(self, name):
        """The directory of one agent under the scratch directory, named with a random token; made on first use."""
        if name not in self.agent_dirs:
            d = os.path.join(self.agents_root(), isolation.token())
            for sub in ("", "home", "tmp") + (() if self.shares_config() else ("claude-config",)):
                os.makedirs(os.path.join(d, sub), mode=0o700, exist_ok=True)
            self.agent_dirs[name] = d
        return self.agent_dirs[name]

    def agents_root(self):
        return os.path.join(self.scratch, "agents")

    def shares_config(self):
        return self.arm in messaging.MESSAGING_ARMS

    def shared_config(self):
        """The CLAUDE_CONFIG_DIR that every agent of a B1 or B1-roles run shares (#1520), or None in any other arm. Native
        messaging finds a peer through the config directory (measured on 2026-11-23: one directory per agent leaves ListAgents
        empty and SendMessage failing), so these agents cannot each have their own. It is made under the run's scratch (never
        ~/.claude), mode 0700, beside the agents' directories and not inside one; the sandbox settings and the scan treat it as
        the agent's own in these two arms and as another's (denied) in every other."""
        if not self.shares_config():
            return None
        if self.shared_cfg is None:
            d = os.path.join(self.agents_root(), "cfg-" + isolation.token())
            os.makedirs(d, mode=0o700, exist_ok=True)
            os.chmod(d, 0o700)
            self.shared_cfg = d
        return self.shared_cfg

    def config_dir(self, name):
        """The CLAUDE_CONFIG_DIR of one agent: the run's shared one in a messaging arm, its own otherwise."""
        return self.shared_config() or os.path.join(self.agent_dir(name), "claude-config")

    def log_path(self, key, kind):
        """The file of a session in the log directory. A driver whose agents poll the log directory (fake) gets the public
        names; a real agent's files have random names until the run ends, so a path learned from the directory's
        listing (it cannot be listed) or guessed (a.stream.jsonl) leads nowhere."""
        if (key, kind) not in self.log_names:
            public = {"stream": "%s.stream.jsonl", "prompt": "%s.prompt.md", "err": "%s.err"}[kind] % key
            self.log_names[(key, kind)] = public if self.driver.lists_logs else isolation.token() + ".log"
            self.log_public[self.log_names[(key, kind)]] = public
        return os.path.join(self.log_dir, self.log_names[(key, kind)])

    def publish_log_names(self):
        """The agents are stopped: the log files take their public names (the metrics read them by those)."""
        for name, public in self.log_public.items():
            if name != public and os.path.exists(os.path.join(self.log_dir, name)):
                os.rename(os.path.join(self.log_dir, name), os.path.join(self.log_dir, public))

    def clone_path(self, name):
        return self.shared_tree if self.is_shared else os.path.join(self.agent_dir(name), "clone")

    def make_clones(self):
        if self.is_shared:
            self.make_shared_tree()
        else:
            for ag in self.fx.agents:
                self.clones[ag["name"]] = self.clone_with_private_files(ag["name"])
        for r in self.role_names:  # a role member has a clone of its own, like a worker, and no private files
            clone = self.clone_path(r)
            self.git(["clone", "-q", "--no-local", self.origin, clone], self.scratch, self.henv)  # #1435: see checks_now
            append_exclude(clone)
            self.clones[r] = clone

    def make_shared_tree(self):
        """T-shared (#1427): ONE working tree, a clone of the bare origin; every agent starts in it. Nothing of the harness
        goes into it: streams, prompts and logs are in log_dir, each agent's HOME and TMPDIR in its own directory."""
        self.git(["clone", "-q", "--no-local", self.origin, self.shared_tree], self.scratch, self.henv)
        append_exclude(self.shared_tree)
        for ag in self.fx.agents:
            self.clones[ag["name"]] = self.shared_tree
            self.agent_dir(ag["name"])

    def clone_with_private_files(self, name):
        clone = self.clone_path(name)
        self.git(["clone", "-q", "--no-local", self.origin, clone], self.scratch, self.henv)  # #1435: see checks_now
        private = self.fx.private_files(name)
        append_exclude(clone, EXCLUDE + "".join("/%s\n" % rel for rel, _ in private))
        for rel, src in private:  # this agent's private knowledge: in its clone only, never committed
            os.makedirs(os.path.dirname(os.path.join(clone, rel)), exist_ok=True)
            shutil.copyfile(src, os.path.join(clone, rel))
        return clone

    def hide_run(self):
        """While agents run nobody can list the directory that holds the agents' directories or the log directory (the harness
        opens what it needs by name). The rest of the scratch directory stays listable: it holds nothing private, and the
        harness's own git and checker runs are not slowed by a hidden parent."""
        os.makedirs(self.agents_root(), exist_ok=True)
        isolation.hide(self.agents_root())
        if self.facts_dir:
            isolation.hide(self.facts_dir)
        if not self.driver.lists_logs:
            isolation.hide(self.log_dir)

    def show_run(self):
        isolation.restore(self.agents_root())
        isolation.restore(self.log_dir)
        if self.facts_dir:
            isolation.restore(self.facts_dir)

    # who may name what ---------------------------------------------------------------------------------------

    def guard_dirs(self):
        a = self.a
        logs_root = os.path.dirname(self.log_dir)
        dirs = [self.scratch, logs_root, self.log_dir, getattr(a, "fixtures_dir", None), getattr(a, "checks_dir", None),
                getattr(a, "scenarios_dir", None), getattr(a, "arms_dir", None), getattr(a, "results_dir", None),
                HERE, REPO_ROOT]
        return [os.path.realpath(d) for d in dirs if d]

    def allowed_dirs(self, name, role=False):
        """What an agent was given besides its own directories: the bare origin, the run store, the staged binary (and the
        restart script for a role member), the S10 directory its injection names."""
        s = self.scratch
        allow = [self.origin, self.khome, self.kkeys, os.path.join(s, "bin"), os.path.join(s, "pwned")]
        if self.a.klatch_bin:
            allow.append(os.path.dirname(self.a.klatch_bin) if os.path.basename(os.path.dirname(self.a.klatch_bin)).startswith("cb-bin-")
                         else self.a.klatch_bin)
        if role and self.restart_dir:
            allow.append(self.restart_dir)
        return allow

    def policy_for(self, name, role=False):
        d = self.agent_dir(name)
        return isolation.Policy(name, (self.clones.get(name) or self.clone_path(name), os.path.join(d, "home"),
                                        os.path.join(d, "tmp")), allow=self.allowed_dirs(name, role) + [self.config_dir(name)] + ([] if self.is_shared else [isolation.Exact(d)]),
                                guard=self.guard_dirs())

    def iso_for(self, name, role=False):
        """What the driver needs to restrict one agent (isolation.sandbox_settings)."""
        d = self.agent_dir(name)
        others = [x for n, x in self.agent_dirs.items() if n != name]
        own = [self.clones.get(name) or self.clone_path(name), os.path.join(d, "home"), os.path.join(d, "tmp"),
               self.config_dir(name)] + [p for p in (os.path.join(self.scratch, "pwned"),) if os.path.isdir(p)]
        shared = [self.origin, self.khome, self.kkeys] + ([self.restart_dir] if role and self.restart_dir else [])
        read = [p for p in self.allowed_dirs(name, role) if p not in shared + own] + isolation.tool_roots()
        logs = [os.path.join(self.log_dir, n) for n in self.log_public if n not in self.log_public.values()]
        return {"mode": self.iso_mode, "root": os.path.realpath(self.scratch), "own": own, "write": shared, "read": read,
                "guard": self.guard_dirs(), "others": others, "settings_dir": d, "log_files": logs}

    def check_role_launch(self):
        """A role member restarts workers by writing into the restart directory with the copied script, from sandboxed Bash:
        refuse the run before any agent starts when either is missing."""
        script = os.path.join(self.scratch, "bin", "restart_member.py")
        missing = [p for p, ok in ((self.restart_dir, os.path.isdir), (script, os.path.isfile)) if not (p and ok(p))]
        if missing:
            raise RuntimeError("role launch: the restart directory or the restart script %s is missing" % script)

    # the scan ------------------------------------------------------------------------------------------------

    def scan_isolation(self):
        """SPEC 11: every tool call of every session is checked for paths outside what the agent was given."""
        total, found = 0, []
        for key, name in self.agent_of.items():
            path = os.path.join(self.log_dir, key + STREAM_SUFFIX)
            try:
                with open(path, errors="replace") as f:
                    lines = f.read().splitlines()
            except OSError:
                continue
            n, v = isolation.scan_lines(lines, self.policy_for(name, key in self.role_keys), session=key)
            total += n
            found += v
        self.iso_result = {"mode": self.iso_mode, "checked_calls": total, "violation_count": len(found), "violations": found[:50]}
        return found

    def config_dir_entries(self, limit=50):
        """What the run left in the shared config directory: {"count", "files": [{"path", "bytes", "sha256"}]} (the first `limit`, sorted),
        a snapshot at the end of the run so a change by one agent that another could read is visible.
        The record is how anyone learns how much state B1 agents share through it (README, "Isolation")."""
        base, found = self.shared_config(), []
        for root, _, files in os.walk(base):
            for f in files:
                p = os.path.join(root, f)
                found.append({"path": os.path.relpath(p, base), "bytes": os.lstat(p).st_size, "sha256": _digest(p)})
        found.sort(key=lambda x: x["path"])
        return {"count": len(found), "files": found[:limit]}

    def note_messaging(self):
        """#1520: a B1 or B1-roles run must show that native messaging worked (messaging.judge). The record gets the evidence;
        a run in which it demonstrably did not work is invalid, with the reason in the notes."""
        alive = {k: (self.started_at[k] - self.t0, self.ended_at.get(k, self.stopped_t) - self.t0) for k in self.procs}
        sessions = {k: dict(s, agent=self.agent_of[k]) for k, s in self.allsums.items() if k in self.agent_of}
        mv = messaging.judge(self.arm, sessions, alive)
        if mv:
            mv["config_dir_entries"] = self.config_dir_entries()
        self.rec["messaging_verified"] = mv
        if mv and mv["status"] == "failed":
            self.rec["valid"] = False
        line = messaging.note_for(mv)
        if line:
            self.notes.append(line)

    def note_isolation(self):
        found = self.scan_isolation()
        self.rec["isolation"] = self.iso_result
        if found:
            self.rec["valid"] = False
            self.notes.append(isolation.describe(found))
