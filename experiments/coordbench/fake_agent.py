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
                                 it the agent stops on a conflict as before
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
            if how == "ours":
                self.bash(["git", "checkout", "--ours", "--", path])
            else:  # union: drop the markers, keep every line of both sides
                with open(path) as f:
                    lines = f.read().splitlines(keepends=True)
                keep = [l for l in lines
                        if not (l.startswith("<<<<<<< ") or l.rstrip("\n") == "=======" or l.startswith(">>>>>>> "))]
                with open(path, "w") as f:
                    f.write("".join(keep))
                self.tool("Edit", {"file_path": path, "resolve": "union"}, "ok")
        self.bash(["git", "add", "--"] + paths)
        self.bash(["git", "commit", "-q", "--no-edit"])
        return True

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
            if f["goal"] == "none" or (forms != "all" and f["id"] not in forms):
                continue
            if f["goal"] == "touch":
                self.bash(["touch", f["path"]])
            else:
                self.step({"op": "append", "path": f["file"], "content": f["line"] + "\n"})

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

    def step(self, st):
        op = st["op"]
        allow = bool(st.get("allow_fail"))
        if op == "write":
            path = st["path"]
            os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
            content = self.fill(st["content"])
            with open(path, "w") as f:
                f.write(content)
            self.tool("Write", {"file_path": path, "content": content}, "ok")
        elif op == "copy_from":
            here = os.path.dirname(os.path.abspath(self.args.script))
            src = st["from"] if os.path.isabs(st["from"]) else os.path.join(here, st["from"])
            os.makedirs(os.path.dirname(os.path.abspath(st["path"])), exist_ok=True)
            with open(src, "rb") as f:
                data = f.read()
            for old, new in st.get("replace", []):
                data = data.replace(old.encode(), new.encode())
            with open(st["path"], "wb") as f:
                f.write(data)
            self.tool("Write", {"file_path": st["path"], "content": data.decode("utf-8", "replace")}, "ok")
        elif op == "append":
            os.makedirs(os.path.dirname(os.path.abspath(st["path"])), exist_ok=True)
            with open(st["path"], "a") as f:
                f.write(st["content"])
            self.tool("Edit", {"file_path": st["path"], "append": st["content"]}, "ok")
        elif op == "replace":
            with open(st["path"]) as f:
                text = f.read()
            if text.count(st["old"]) != 1:
                self.die("replace: the text to replace occurs %d times in %s" % (text.count(st["old"]), st["path"]))
            with open(st["path"], "w") as f:
                f.write(text.replace(st["old"], st["new"]))
            self.tool("Edit", {"file_path": st["path"], "old_string": st["old"], "new_string": st["new"]}, "ok")
        elif op == "git":
            self.bash(["git"] + st["args"], allow)
        elif op == "sync":
            msg = st.get("message", "work")
            self.bash(["git", "add", "-A"], allow)
            self.bash(["git", "commit", "-q", "-m", msg], True)
            resolve = st.get("resolve")
            for _ in range(8):
                rc, _ = self.bash(["git", "pull", "--no-rebase", "--no-edit", "origin", "main"], allow or bool(resolve))
                if rc != 0:
                    if not resolve:
                        return
                    if not self.resolve_conflicts(resolve):
                        self.die("pull failed and there is no conflict to resolve")
                rc, _ = self.bash(["git", "push", "origin", "HEAD:main"], True)
                if rc == 0:
                    return
                time.sleep(0.2)
            if not allow:
                self.die("push kept losing the race")
        elif op == "klatch":
            if not self.args.klatch_bin:
                self.die("the script runs klatch but --klatch-bin was not given")
            _, out = self.bash([self.args.klatch_bin] + [self.fill(a) for a in st["args"]], allow)
            if st.get("capture"):  # the number of the ask this command made ("asked #N")
                m = re.search(r"asked #(\d+)", out)
                self.vars[st["capture"]] = m.group(1) if m else ""
        elif op == "set_var_from_file":
            with open(st["path"]) as f:
                self.vars[st["key"]] = f.read().strip()
        elif op == "send":
            self.tool("SendMessage", {"to": st["to"], "message": st["text"]}, "sent")
        elif op == "dump_specs":
            _, tracked = self.run_cmd(["git", "ls-files", "SPEC-*"])
            _, status = self.run_cmd(["git", "status", "--porcelain"])
            facts = {"files": sorted(n for n in os.listdir(".") if n.startswith("SPEC-")),
                     "tracked": tracked.split(), "status": status.splitlines()}
            with open(os.path.join(self.args.out, st["path"]), "w") as f:
                json.dump(facts, f)
        elif op == "dump_board":
            with open("TASKS.md") as src, open(os.path.join(self.args.out, st["path"]), "w") as f:
                f.write(src.read())
        elif op == "read":
            with open(st["path"]) as f:
                self.tool("Read", {"file_path": st["path"]}, f.read())
        elif op == "run":
            self.bash([self.fill(a) for a in st["argv"]], allow)
        elif op == "hook":
            emit({"type": "system", "subtype": "hook_response", "hook_event": "SessionStart", "output": self.fill(st["text"])})
        elif op == "say":
            self.assistant({"type": "text", "text": st["text"]})
        elif op == "cost":
            self.cost = float(st["usd"])
            emit({"type": "system", "subtype": "fake_cost", "usd": self.cost})
        elif op == "sleep":
            time.sleep(float(st["s"]))
        elif op == "hang":
            time.sleep(3600)
        elif op == "exit":
            sys.exit(int(st.get("code", 1)))
        elif op == "poll":
            for _ in range(int(st.get("n", 1))):
                self.tool("Bash", {"command": st.get("command", "sleep 5")}, "")
        elif op == "wait_origin_has":
            deadline = time.time() + float(st.get("timeout", 30))
            while time.time() < deadline:
                subprocess.run(["git", "fetch", "-q", "origin"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                rc, out = self.run_cmd(["git", "show", "origin/main:%s" % st["path"]])
                if rc == 0 and st.get("contains", "") in out:
                    self.tool("Bash", {"command": "git fetch origin"}, "found %s" % st["path"])
                    return
                time.sleep(0.1)
            self.die("origin never had %s" % st["path"])
        elif op == "wait_file":
            deadline = time.time() + float(st.get("timeout", 30))
            target = os.path.join(self.args.out, st["path"])
            while time.time() < deadline:
                if os.path.exists(target):
                    self.tool("Bash", {"command": "ls"}, "found %s" % st["path"])
                    return
                time.sleep(0.05)
            self.die("%s never appeared" % st["path"])
        elif op == "send":
            self.tool("SendMessage", {"to": st["to"], "message": self.fill(st["text"])}, "sent")
        elif op == "dump_env":
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
            with open(os.path.join(self.args.out, st["path"]), "w") as f:
                json.dump(facts, f)
        elif op == "read_into":
            with open(st["path"]) as f:
                text = f.read()
            self.vars[st["key"]] = text
            self.tool("Read", {"file_path": st["path"]}, text)
        elif op == "write_from_var":
            text = self.vars[st["key"]]
            for old, new in st.get("replace", []):
                if text.count(old) != 1:
                    self.die("write_from_var: the text to replace occurs %d times in the kept copy" % text.count(old))
                text = text.replace(old, new)
            text += st.get("append", "")
            with open(st["path"], "w") as f:
                f.write(text)
            self.tool("Write", {"file_path": st["path"], "content": text}, "ok")
        elif op == "signal":
            self.signal(st["name"])
        elif op == "wait_signal":
            self.wait_signal(st["name"], int(st.get("parties", 1)), float(st.get("timeout", 30)))
        elif op == "barrier":
            self.signal(st["name"])
            self.wait_signal(st["name"], int(st["parties"]), float(st.get("timeout", 30)))
        elif op == "obey":
            self.obey(st.get("forms", "all"))
        elif op == "final":
            text = st["text"]
            if "{{TOKEN:" in text:
                for f in self.plan()["forms"]:
                    text = text.replace("{{TOKEN:%s}}" % f["id"], f["token"])
            self.final = text
        else:
            self.die("unknown op %s" % op)

    def run(self, script):
        key = self.args.name if self.args.session == 1 else "%s#%d" % (self.args.name, self.args.session)
        steps = script.get("agents", {}).get(key, script.get("default", []))
        emit({"type": "system", "subtype": "init", "model": "claude-fake", "session_id": "fake-" + self.args.name})
        for st in steps:
            if st.get("arms") and self.args.arm not in st["arms"]:
                continue
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
    args = p.parse_args()
    with open(args.script) as f:
        script = json.load(f)
    Agent(args).run(script)


if __name__ == "__main__":
    main()
