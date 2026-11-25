#!/usr/bin/env python3
"""A scripted, deterministic fake agent for the harness tests (#1364). It uses no model and no network.

It reads a JSON script and performs the steps for its name: file edits, git, klatch commands, waiting, failures. As it
goes it prints stream-json events in the shape `claude -p --output-format stream-json` prints (init, assistant tool_use,
user tool_result, result), so the harness parses a fake run exactly as it parses a real one.

    fake_agent.py --script FILE --name NAME --arm ARM --out DIR [--room ROOM] [--klatch-bin PATH] [--session N]

--session 2 is the restart of an agent after its context was cleared (S9 resume): it runs the steps under the key "NAME#2"
(the default steps when there are none), a separate process that remembers nothing of session 1 except what is on disk.

Script: {"agents": {"<name>": [step, ...]}, "default": [step, ...]}. A step may carry "arms": ["K1", ...] to run only in
those arms, and "allow_fail": true to go on after a failing command. Steps (op):

  write {path, content}          write a file in the working directory
  git {args}                     run git; a nonzero exit ends the agent with exit 1 unless allow_fail
  copy_from {from, path, replace?}  copy a file ("from" is absolute or relative to the script's directory) to path;
                                 replace is a list of [old, new] text pairs applied to the copy (a fixed wrong value)
  send {to, text}                a SendMessage tool call (no delivery: the fake agent knows its script)
  dump_specs {path}              write the SPEC-* files in the working directory, the tracked ones and git status as JSON
                                 to OUT/path
  dump_board {path}              write TASKS.md as it is in the working directory to OUT/path
  append {path, content}         append text to a file (created when absent)
  replace {path, old, new}       replace the one occurrence of old in a file with new; the agent fails when old is absent
                                 or occurs more than once (an edit of a file as it is now, not a copy of a stale one)
  sync {message, resolve?}       git add -A, commit, pull --no-rebase, push to origin main (retries a lost race).
                                 resolve: "union" (keep both sides of every conflicted file) or "ours" (keep this
                                 agent's side, dropping the other's lines) resolves a conflicted pull and goes on; without
                                 it the agent stops on a conflict as before; retries (default 8) is how many times a lost push
                                 race is retried (a team of 16 racing on one file needs more)
  klatch {args}                  run the klatch binary given by --klatch-bin ({{ROOM}} and {{NAME}} are filled in)
  say {text}                     an assistant text message
  read {path}                    a Read tool call on a file (its text is the result)
  run {argv}                     run any command (a Bash tool call); a nonzero exit ends the agent unless allow_fail
  hook {text}                    a SessionStart style hook_response event with this output (what a hook injects)
  cost {usd}                     report cumulative spend so far (a fake_cost event)
  sleep {s}, hang                wait; hang waits until the harness kills it
  exit {code}                    stop at once with that exit code, with no result message (a crashed agent)
  poll {n, command?}             n polling turns: n Bash calls of the command (default `sleep 5`) with empty output; no real
                                 waiting (the scenario S3 tests use it for a blocked agent that polls many times)
  wait_origin_has {path, contains?, timeout}  fetch until origin/main has the file (and, when given, text in it)
  wait_file {path, timeout}      wait until OUT/path exists (the harness writes kill.json there when it kills the victim of
                                 scenario S6: a scripted survivor "notices" once it appears; a real agent never reads it)
  send {to, text}                a SendMessage tool call (nothing is delivered: the fake team has no messaging)
  dump_env {path}                write the isolation facts (not the environment) as JSON to OUT/path
  obey {forms}                   obey injections (S10): "all" or a list of form ids; reads the run's plan from OUT/injection-plan.json
                                 and does what each form asked (append the canary line, append the stop marker, touch the file)
  final {text}                   the final message; {{TOKEN:form}} is filled with the run's token for that injection form

Shared working tree (#1427, topology T-shared): several fake agents run in ONE directory, so a lost update needs a fixed
interleaving. These ops give it, with files in OUT (the per-agent log directory, outside the tree):

  read_into {path, key}          a Read tool call whose text is kept as the variable key (a stale copy, used later)
  write_from_var {path, key, replace?, append?}  a Write tool call of the kept text with [old, new] pairs applied and text
                                 appended: the whole file as that agent saw it, silently overwriting whatever changed since
  signal {name}                  mark that this agent reached name
  wait_signal {name, parties?, timeout?}  wait until parties (default 1) agents have signalled name
  barrier {name, parties}        signal, then wait_signal: every party waits for all the others

Role arms (#1421). In K3 and B1-roles a step with "arms": ["K2"] (["B1"]) also runs: the role arm is the base arm plus roles.
The worker steps change in a role arm only for `sync`: a worker does not push to main, it pushes its own branch, announces
`ready: NAME` (a klatch post in K3, a SendMessage to the integrator in B1-roles) and waits (merge_wait_s, default 30) until
the integrator has merged it; a sync with "direct": true keeps the plain push to main (a board line is not code). The role members' ops:

  integrate {branches, test?, resolve?, timeout?}  the integrator: for each branch in this order wait until origin has it,
                                 announce `integrating: B`, merge it into main, run the test argv, push main and announce
                                 `merged: B`; a conflict (unless resolve is "union") or a failing test aborts and announces
                                 `dropped: B conflict` or `dropped: B checks-failed`
  restart {member}               the supervisor: run restart_member.py MEMBER (a Bash tool call); allow_fail goes on after a refusal
"""
import argparse
import json
import os
import re
import shlex
import stat
import subprocess
import sys
import time

ROLE_ARMS = {"K3": "K2", "B1-roles": "B1"}  # the base arm a role arm adds roles to (roles.py)
HERE = os.path.dirname(os.path.abspath(__file__))


def emit(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


class Agent:
    def __init__(self, args):
        self.args = args
        self.turn = 0
        self.cost = 0.0
        self.final = "DONE %s" % args.name
        self.vars = {}  # values a step captured ("capture") or read from a file ("set_var_from_file"), used as {{var:KEY}}

    def assistant(self, block):
        self.turn += 1
        emit({"type": "assistant", "message": {"id": "fake-%s-%d" % (self.args.name, self.turn), "model": "claude-fake",
                                               "content": [block], "usage": {"input_tokens": 10, "output_tokens": 5}}})

    def tool(self, name, inp, output):
        tid = "toolu_%s_%d" % (self.args.name, self.turn + 1)
        self.assistant({"type": "tool_use", "id": tid, "name": name, "input": inp})
        emit({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": tid, "content": output}]}})

    def run_cmd(self, argv, shown=None):
        p = subprocess.run(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        return p.returncode, p.stdout

    def bash(self, argv, allow_fail=False):
        rc, out = self.run_cmd(argv)
        self.tool("Bash", {"command": shlex.join(argv)}, out)
        if rc != 0 and not allow_fail:
            self.die("command failed (%d): %s" % (rc, shlex.join(argv)))
        return rc, out

    def die(self, why):
        emit({"type": "result", "subtype": "error_during_execution", "is_error": True, "num_turns": self.turn,
              "total_cost_usd": self.cost, "result": why})
        sys.exit(1)

    def resolve_conflicts(self, how):
        """Resolve a conflicted merge in the working tree and commit it. False when there are no unmerged paths."""
        rc, out = self.run_cmd(["git", "diff", "--name-only", "--diff-filter=U"])
        paths = [p for p in out.splitlines() if p]
        if rc != 0 or not paths:
            return False
        for path in paths:
            self.resolve_path(path, how)
        self.bash(["git", "add", "--"] + paths)
        self.bash(["git", "commit", "-q", "--no-edit"])
        return True

    def resolve_path(self, path, how):
        if how == "ours":
            self.bash(["git", "checkout", "--ours", "--", path])
            return
        # union: drop the markers, keep every line of both sides
        with open(path) as f:
            lines = f.read().splitlines(keepends=True)
        keep = [l for l in lines
                if not (l.startswith("<<<<<<< ") or l.rstrip("\n") == "=======" or l.startswith(">>>>>>> "))]
        with open(path, "w") as f:
            f.write("".join(keep))
        self.tool("Edit", {"file_path": path, "resolve": "union"}, "ok")

    def fill(self, text):
        text = text.replace("{{ROOM}}", self.args.room).replace("{{NAME}}", self.args.name)
        for k, v in self.vars.items():
            text = text.replace("{{var:%s}}" % k, v)
        return text

    def plan(self):
        with open(os.path.join(self.args.out, "injection-plan.json")) as f:
            return json.load(f)

    def obey(self, forms):
        for f in self.plan()["forms"]:
            if not self.wants_form(f, forms):
                continue
            if f["goal"] == "touch":
                self.bash(["touch", f["path"]])
            else:
                self.step({"op": "append", "path": f["file"], "content": f["line"] + "\n"})

    @staticmethod
    def wants_form(f, forms):
        """An injection form the agent obeys: one with a goal, and listed in forms (or all of them)."""
        return f["goal"] != "none" and (forms == "all" or f["id"] in forms)

    def signal(self, name):
        with open(os.path.join(self.args.out, "sig-%s.%s" % (name, self.args.name)), "w") as f:
            f.write("1")

    def wait_signal(self, name, parties, timeout):
        deadline = time.time() + timeout
        prefix = "sig-%s." % name
        while time.time() < deadline:
            if sum(1 for n in os.listdir(self.args.out) if n.startswith(prefix)) >= parties:
                return
            time.sleep(0.02)
        self.die("signal %s never reached %d parties" % (name, parties))

    # role arms (#1421) ---------------------------------------------------------------------------------------

    def role_arm(self):
        return self.args.arm in ROLE_ARMS

    def announce(self, text, to=None):
        """Say a role-arm line the way the arm does: a klatch post in K3, a SendMessage tool call in B1-roles."""
        if self.args.arm.startswith("K"):
            if self.args.klatch_bin:
                self.bash([self.args.klatch_bin, "post", self.args.room, text], True)
        else:
            self.tool("SendMessage", {"to": to or "integrator", "message": text}, "sent")

    def role_sync(self, st, allow):
        """A worker's publish in a role arm: its own branch, ready: NAME, then wait until the integrator merged it."""
        name = self.args.name
        self.bash(["git", "add", "-A"], allow)
        self.bash(["git", "commit", "-q", "-m", st.get("message", "work")], True)
        self.bash(["git", "pull", "--no-rebase", "--no-edit", "origin", "main"], True)
        rc, _ = self.bash(["git", "push", "origin", "HEAD:refs/heads/%s" % name], True)
        if rc != 0 and not allow:
            self.die("could not push the branch %s" % name)
        self.announce("ready: %s" % name)
        deadline = time.time() + float(st.get("merge_wait_s", 30))
        while time.time() < deadline:
            subprocess.run(["git", "fetch", "-q", "origin"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            rc, _ = self.run_cmd(["git", "merge-base", "--is-ancestor", "HEAD", "origin/main"])
            if rc == 0:
                self.tool("Bash", {"command": "git fetch origin"}, "merged: %s" % name)
                self.bash(["git", "pull", "--no-rebase", "--no-edit", "origin", "main"], True)
                return
            time.sleep(0.1)
        if not allow:
            self.die("%s was never merged" % name)

    def integrate(self, st):
        """The integrator: merge the listed branches into main one at a time, in this order."""
        for branch in st["branches"]:
            self.wait_for_branch(branch, float(st.get("timeout", 40)))
            self.announce("integrating: %s" % branch, branch)
            self.bash(["git", "checkout", "-q", "-B", "main", "origin/main"])
            dropped = self.merge_and_push(branch, st)
            if dropped:
                self.announce("dropped: %s %s" % (branch, dropped), branch)
            else:
                self.announce("merged: %s" % branch, branch)

    def wait_for_branch(self, branch, timeout):
        deadline = time.time() + timeout
        while True:
            subprocess.run(["git", "fetch", "-q", "origin"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            rc, _ = self.run_cmd(["git", "rev-parse", "--verify", "-q", "origin/%s" % branch])
            if rc == 0:
                return
            if time.time() > deadline:
                self.die("branch %s never appeared" % branch)
            time.sleep(0.1)

    def merge_and_push(self, branch, st):
        """Merge one branch into main and push it. None when merged; else why it was dropped (conflict, checks-failed, error)."""
        rc, _ = self.bash(["git", "merge", "--no-ff", "--no-edit", "origin/%s" % branch], True)
        if rc != 0:
            if not (st.get("resolve") == "union" and self.resolve_conflicts("union")):
                self.bash(["git", "merge", "--abort"], True)
                return "conflict"
        test = st.get("test")
        if test and self.bash(list(test), True)[0] != 0:
            self.bash(["git", "reset", "-q", "--hard", "origin/main"])
            return "checks-failed"
        rc, _ = self.bash(["git", "push", "origin", "HEAD:main"], True)
        if rc != 0:
            self.bash(["git", "reset", "-q", "--hard", "origin/main"])
            return "error"
        return None

    def write_file(self, path, mode, text):
        with open(path, mode) as f:
            f.write(text)

    def ensure_dir_of(self, path):
        os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)

    def op_write(self, st, allow):
        path = st["path"]
        self.ensure_dir_of(path)
        content = self.fill(st["content"])
        self.write_file(path, "w", content)
        self.tool("Write", {"file_path": path, "content": content}, "ok")

    def op_copy_from(self, st, allow):
        here = os.path.dirname(os.path.abspath(self.args.script))
        src = st["from"] if os.path.isabs(st["from"]) else os.path.join(here, st["from"])
        self.ensure_dir_of(st["path"])
        with open(src, "rb") as f:
            data = f.read()
        for old, new in st.get("replace", []):
            data = data.replace(old.encode(), new.encode())
        with open(st["path"], "wb") as f:
            f.write(data)
        self.tool("Write", {"file_path": st["path"], "content": data.decode("utf-8", "replace")}, "ok")

    def op_append(self, st, allow):
        self.ensure_dir_of(st["path"])
        self.write_file(st["path"], "a", st["content"])
        self.tool("Edit", {"file_path": st["path"], "append": st["content"]}, "ok")

    def op_replace(self, st, allow):
        with open(st["path"]) as f:
            text = f.read()
        if text.count(st["old"]) != 1:
            self.die("replace: the text to replace occurs %d times in %s" % (text.count(st["old"]), st["path"]))
        self.write_file(st["path"], "w", text.replace(st["old"], st["new"]))
        self.tool("Edit", {"file_path": st["path"], "old_string": st["old"], "new_string": st["new"]}, "ok")

    def op_git(self, st, allow):
        self.bash(["git"] + st["args"], allow)

    def op_integrate(self, st, allow):
        self.integrate(st)

    def op_restart(self, st, allow):
        self.bash([sys.executable, self.restart_script(), st["member"]], allow)

    def restart_script(self):
        """The copy of restart_member.py the harness put in the run's bin directory (the prompt of a real supervisor names that
        copy, never a path in the repository, #1519); the repository's own file only when the run has no restart directory (#1471)."""
        d = os.environ.get("COORDBENCH_RESTART_DIR")
        copy = os.path.join(os.path.dirname(d), "bin", "restart_member.py") if d else ""
        return copy if copy and os.path.isfile(copy) else os.path.join(HERE, "restart_member.py")

    def op_sync(self, st, allow):
        if self.role_arm() and not st.get("direct"):
            self.role_sync(st, allow)
        else:
            self.plain_sync(st, allow)

    def plain_sync(self, st, allow):
        self.bash(["git", "add", "-A"], allow)
        self.bash(["git", "commit", "-q", "-m", st.get("message", "work")], True)
        resolve = st.get("resolve")
        for _ in range(int(st.get("retries", 8))):
            rc, _ = self.bash(["git", "pull", "--no-rebase", "--no-edit", "origin", "main"], allow or bool(resolve))
            if rc != 0 and not self.settle_failed_pull(resolve):
                return
            rc, _ = self.bash(["git", "push", "origin", "HEAD:main"], True)
            if rc == 0:
                return
            time.sleep(0.2)
        if not allow:
            self.die("push kept losing the race")

    def settle_failed_pull(self, resolve):
        """After a failed pull in sync: False to stop (no resolve asked), True to go on once the conflict is resolved."""
        if not resolve:
            return False
        if not self.resolve_conflicts(resolve):
            self.die("pull failed and there is no conflict to resolve")
        return True

    def op_klatch(self, st, allow):
        if not self.args.klatch_bin:
            self.die("the script runs klatch but --klatch-bin was not given")
        _, out = self.bash([self.args.klatch_bin] + [self.fill(a) for a in st["args"]], allow)
        if st.get("capture"):  # the number of the ask this command made ("asked #N")
            m = re.search(r"asked #(\d+)", out)
            self.vars[st["capture"]] = m.group(1) if m else ""

    def op_set_var_from_file(self, st, allow):
        with open(st["path"]) as f:
            self.vars[st["key"]] = f.read().strip()

    def op_send(self, st, allow):
        self.tool("SendMessage", {"to": st["to"], "message": st["text"]}, "sent")

    def op_dump_specs(self, st, allow):
        _, tracked = self.run_cmd(["git", "ls-files", "SPEC-*"])
        _, status = self.run_cmd(["git", "status", "--porcelain"])
        facts = {"files": sorted(n for n in os.listdir(".") if n.startswith("SPEC-")),
                 "tracked": tracked.split(), "status": status.splitlines()}
        with open(os.path.join(self.args.out, st["path"]), "w") as f:
            json.dump(facts, f)

    def op_dump_board(self, st, allow):
        with open("TASKS.md") as src, open(os.path.join(self.args.out, st["path"]), "w") as f:
            f.write(src.read())

    def op_read(self, st, allow):
        with open(st["path"]) as f:
            self.tool("Read", {"file_path": st["path"]}, f.read())

    def op_run(self, st, allow):
        self.bash([self.fill(a) for a in st["argv"]], allow)

    def op_hook(self, st, allow):
        emit({"type": "system", "subtype": "hook_response", "hook_event": "SessionStart", "output": self.fill(st["text"])})

    def op_say(self, st, allow):
        self.assistant({"type": "text", "text": st["text"]})

    def op_cost(self, st, allow):
        self.cost = float(st["usd"])
        emit({"type": "system", "subtype": "fake_cost", "usd": self.cost})

    def op_sleep(self, st, allow):
        time.sleep(float(st["s"]))

    def op_hang(self, st, allow):
        time.sleep(3600)

    def op_exit(self, st, allow):
        sys.exit(int(st.get("code", 1)))

    def op_poll(self, st, allow):
        for _ in range(int(st.get("n", 1))):
            self.tool("Bash", {"command": st.get("command", "sleep 5")}, "")

    def op_wait_origin_has(self, st, allow):
        deadline = time.time() + float(st.get("timeout", 30))
        while time.time() < deadline:
            subprocess.run(["git", "fetch", "-q", "origin"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            rc, out = self.run_cmd(["git", "show", "origin/main:%s" % st["path"]])
            if rc == 0 and st.get("contains", "") in out:
                self.tool("Bash", {"command": "git fetch origin"}, "found %s" % st["path"])
                return
            time.sleep(0.1)
        self.die("origin never had %s" % st["path"])

    def op_wait_file(self, st, allow):
        deadline = time.time() + float(st.get("timeout", 30))
        target = os.path.join(self.args.out, st["path"])
        while time.time() < deadline:
            if os.path.exists(target):
                self.tool("Bash", {"command": "ls"}, "found %s" % st["path"])
                return
            time.sleep(0.05)
        self.die("%s never appeared" % st["path"])

    def isolation_facts(self):
        keys = ("HOME", "KLATCH_HOME", "KLATCH_KEY_DIR", "KLATCH_NAME")
        facts = {k: os.environ.get(k) for k in keys}
        facts["modes"] = {k: stat.S_IMODE(os.stat(os.environ[k]).st_mode) for k in ("KLATCH_HOME", "KLATCH_KEY_DIR")
                          if os.environ.get(k) and os.path.exists(os.environ[k])}
        facts["cwd"] = os.getcwd()
        facts["identity"] = {k: os.environ.get(k) for k in ("GIT_AUTHOR_NAME", "GIT_COMMITTER_NAME", "TMPDIR")}
        facts["out_inside_cwd"] = os.path.realpath(self.args.out).startswith(os.path.realpath(os.getcwd()) + os.sep)
        tree = []
        for d, dirs, names in os.walk("."):
            dirs[:] = [x for x in dirs if x != ".git"]
            tree += [os.path.relpath(os.path.join(d, n)) for n in names]
        facts["tree_files"] = sorted(tree)
        facts["has_anthropic_key"] = "ANTHROPIC_API_KEY" in os.environ
        facts["settings_local_exists"] = os.path.exists(os.path.join(".claude", "settings.local.json"))
        return facts

    def op_dump_env(self, st, allow):
        with open(os.path.join(self.args.out, st["path"]), "w") as f:
            json.dump(self.isolation_facts(), f)

    def op_read_into(self, st, allow):
        with open(st["path"]) as f:
            text = f.read()
        self.vars[st["key"]] = text
        self.tool("Read", {"file_path": st["path"]}, text)

    def op_write_from_var(self, st, allow):
        text = self.vars[st["key"]]
        for old, new in st.get("replace", []):
            if text.count(old) != 1:
                self.die("write_from_var: the text to replace occurs %d times in the kept copy" % text.count(old))
            text = text.replace(old, new)
        text += st.get("append", "")
        self.write_file(st["path"], "w", text)
        self.tool("Write", {"file_path": st["path"], "content": text}, "ok")

    def op_signal(self, st, allow):
        self.signal(st["name"])

    def op_wait_signal(self, st, allow):
        self.wait_signal(st["name"], int(st.get("parties", 1)), float(st.get("timeout", 30)))

    def op_barrier(self, st, allow):
        self.signal(st["name"])
        self.wait_signal(st["name"], int(st["parties"]), float(st.get("timeout", 30)))

    def op_obey(self, st, allow):
        self.obey(st.get("forms", "all"))

    def op_final(self, st, allow):
        text = st["text"]
        if "{{TOKEN:" in text:
            for f in self.plan()["forms"]:
                text = text.replace("{{TOKEN:%s}}" % f["id"], f["token"])
        self.final = text

    def step(self, st):
        op = st["op"]
        handler = getattr(self, "op_" + op, None) if isinstance(op, str) and re.fullmatch(r"\w+", op) else None
        if handler is None:
            self.die("unknown op %s" % op)
        handler(st, bool(st.get("allow_fail")))

    def in_arm(self, st):
        """A step with no "arms" runs everywhere; else in a listed arm, or in a role arm whose base arm is listed."""
        arms = st.get("arms")
        return not arms or self.args.arm in arms or ROLE_ARMS.get(self.args.arm) in arms

    def run(self, script):
        key = self.args.name if self.args.session == 1 else "%s#%d" % (self.args.name, self.args.session)
        steps = script.get("agents", {}).get(key, script.get("default", []))
        emit({"type": "system", "subtype": "init", "model": "claude-fake", "session_id": "fake-" + self.args.name})
        for st in steps:
            if self.in_arm(st):
                self.step(st)
        self.assistant({"type": "text", "text": self.final})
        emit({"type": "result", "subtype": "success", "is_error": False, "num_turns": self.turn,
              "total_cost_usd": self.cost, "duration_ms": 0, "result": self.final,
              "usage": {"input_tokens": 10 * self.turn, "output_tokens": 5 * self.turn, "cache_read_input_tokens": 0,
                        "cache_creation_input_tokens": 0}})


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--script", required=True)
    p.add_argument("--name", required=True)
    p.add_argument("--arm", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--room", default="lobby")
    p.add_argument("--klatch-bin")
    p.add_argument("--session", type=int, default=1)
    p.add_argument("--role", help="integrator or supervisor: this agent is a role member of a role arm (#1421)")
    args = p.parse_args()
    with open(args.script) as f:
        script = json.load(f)
    Agent(args).run(script)


if __name__ == "__main__":
    main()
