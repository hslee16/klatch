"""The isolation part of one run (#1519), a mixin of runexec.RunExecution: the per-agent directories, who may name what,
hiding the scratch and log directories while agents run, and the post-run scan of every stream (SPEC 11).

Layout of the run scratch directory (mode 0300 while agents run, see isolation.py):

    <scratch>/<token>/clone    an agent's clone (T-clone; in T-shared the one shared tree is <scratch>/shared)
    <scratch>/<token>/home     its HOME;  tmp: its TMPDIR;  claude-config: its CLAUDE_CONFIG_DIR
    <scratch>/origin.git, klatch-home, klatch-keys, bin (the restart script), restart, pwned (S10)
"""
import os
import shutil

import isolation
from runenv import EXCLUDE, append_exclude

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.dirname(os.path.dirname(HERE))
STREAM_SUFFIX = ".stream.jsonl"


class IsolationSupport:
    def init_isolation(self):
        self.agent_dirs = {}
        self.iso_mode = getattr(self.a, "isolation", "sandbox")
        self.iso_result = None

    # layout --------------------------------------------------------------------------------------------------

    def agent_dir(self, name):
        """The directory of one agent under the scratch directory, named with a random token; made on first use."""
        if name not in self.agent_dirs:
            d = os.path.join(self.scratch, isolation.token())
            for sub in ("", "home", "tmp", "claude-config"):
                os.makedirs(os.path.join(d, sub), mode=0o700, exist_ok=True)
            self.agent_dirs[name] = d
        return self.agent_dirs[name]

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
        """While agents run nobody can list the scratch directory or the log directory (the harness opens what it needs by
        name)."""
        isolation.hide(self.scratch)
        isolation.hide(self.log_dir)

    def show_run(self):
        isolation.restore(self.scratch)
        isolation.restore(self.log_dir)

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
        return isolation.Policy(name, self.clones.get(name) or self.clone_path(name), os.path.join(d, "home"),
                                os.path.join(d, "tmp"), allow=self.allowed_dirs(name, role) + [os.path.join(d, "claude-config")],
                                guard=self.guard_dirs())

    def iso_for(self, name, role=False):
        """What the driver needs to restrict one agent (isolation.sandbox_settings)."""
        d = self.agent_dir(name)
        others = [x for n, x in self.agent_dirs.items() if n != name]
        own = [self.clones.get(name) or self.clone_path(name), os.path.join(d, "home"), os.path.join(d, "tmp"),
               os.path.join(d, "claude-config")] + [p for p in (os.path.join(self.scratch, "pwned"),) if os.path.isdir(p)]
        shared = [self.origin, self.khome, self.kkeys]
        read = [p for p in self.allowed_dirs(name, role) if p not in shared + own]
        return {"mode": self.iso_mode, "root": os.path.realpath(self.scratch), "own": own, "write": shared, "read": read,
                "guard": self.guard_dirs(), "others": others, "settings_dir": d}

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

    def note_isolation(self):
        found = self.scan_isolation()
        self.rec["isolation"] = self.iso_result
        if found:
            self.rec["valid"] = False
            self.notes.append(isolation.describe(found))
