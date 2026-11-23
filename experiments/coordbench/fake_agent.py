#!/usr/bin/env python3
"""A scripted, deterministic fake agent for the harness tests (#1364). It uses no model and no network.

It reads a JSON script and performs the steps for its name: file edits, git, klatch commands, waiting, failures. As it
goes it prints stream-json events in the shape `claude -p --output-format stream-json` prints (init, assistant tool_use,
user tool_result, result), so the harness parses a fake run exactly as it parses a real one.

    fake_agent.py --script FILE --name NAME --arm ARM --out DIR [--room ROOM] [--klatch-bin PATH]

Script: {"agents": {"<name>": [step, ...]}, "default": [step, ...]}. A step may carry "arms": ["K1", ...] to run only in
those arms, and "allow_fail": true to go on after a failing command. Steps (op):

  write {path, content}          write a file in the working directory
  git {args}                     run git; a nonzero exit ends the agent with exit 1 unless allow_fail
  sync {message}                 git add -A, commit, pull --no-rebase, push origin HEAD:main (retries a lost race)
  klatch {args}                  run the klatch binary given by --klatch-bin ({{ROOM}} and {{NAME}} are filled in)
  say {text}                     an assistant text message
  cost {usd}                     report cumulative spend so far (a fake_cost event)
  sleep {s}, hang                wait; hang waits until the harness kills it
  exit {code}                    stop at once with that exit code, with no result message (a crashed agent)
  wait_origin_has {path, timeout}  fetch until origin/main has the file
  dump_env {path}                write the isolation facts (not the environment) as JSON to OUT/path
  final {text}                   the final message
"""
import argparse
import json
import os
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

    def fill(self, text):
        return text.replace("{{ROOM}}", self.args.room).replace("{{NAME}}", self.args.name)

    def step(self, st):
        op = st["op"]
        allow = bool(st.get("allow_fail"))
        if op == "write":
            path = st["path"]
            os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
            with open(path, "w") as f:
                f.write(st["content"])
            self.tool("Write", {"file_path": path, "content": st["content"]}, "ok")
        elif op == "git":
            self.bash(["git"] + st["args"], allow)
        elif op == "sync":
            msg = st.get("message", "work")
            self.bash(["git", "add", "-A"], allow)
            self.bash(["git", "commit", "-q", "-m", msg], True)
            for _ in range(6):
                rc, _ = self.bash(["git", "pull", "--no-rebase", "--no-edit", "origin", "main"], allow)
                if rc != 0:
                    return
                rc, _ = self.bash(["git", "push", "origin", "HEAD:main"], True)
                if rc == 0:
                    return
                time.sleep(0.2)
            if not allow:
                self.die("push kept losing the race")
        elif op == "klatch":
            if not self.args.klatch_bin:
                self.die("the script runs klatch but --klatch-bin was not given")
            self.bash([self.args.klatch_bin] + [self.fill(a) for a in st["args"]], allow)
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
        elif op == "wait_origin_has":
            deadline = time.time() + float(st.get("timeout", 30))
            while time.time() < deadline:
                subprocess.run(["git", "fetch", "-q", "origin"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                rc, _ = self.run_cmd(["git", "cat-file", "-e", "origin/main:%s" % st["path"]])
                if rc == 0:
                    self.tool("Bash", {"command": "git fetch origin"}, "found %s" % st["path"])
                    return
                time.sleep(0.1)
            self.die("origin never had %s" % st["path"])
        elif op == "dump_env":
            keys = ("HOME", "KLATCH_HOME", "KLATCH_KEY_DIR", "KLATCH_NAME")
            facts = {k: os.environ.get(k) for k in keys}
            facts["modes"] = {k: stat.S_IMODE(os.stat(os.environ[k]).st_mode) for k in ("KLATCH_HOME", "KLATCH_KEY_DIR")
                              if os.environ.get(k) and os.path.exists(os.environ[k])}
            facts["cwd"] = os.getcwd()
            facts["has_anthropic_key"] = "ANTHROPIC_API_KEY" in os.environ
            facts["settings_local_exists"] = os.path.exists(os.path.join(".claude", "settings.local.json"))
            with open(os.path.join(self.args.out, st["path"]), "w") as f:
                json.dump(facts, f)
        elif op == "final":
            self.final = st["text"]
        else:
            self.die("unknown op %s" % op)

    def run(self, script):
        steps = script.get("agents", {}).get(self.args.name, script.get("default", []))
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
    args = p.parse_args()
    with open(args.script) as f:
        script = json.load(f)
    Agent(args).run(script)


if __name__ == "__main__":
    main()
