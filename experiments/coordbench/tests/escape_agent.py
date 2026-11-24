#!/usr/bin/env python3
"""A fake agent that tries every way out of its own directories that is known (#1519), for the harness self-test.

It prints stream-json like a real agent (one tool_use event per attempt, so the post-run scan has something to read) and
really performs each attempt, then writes <out>/<name>.escape.json: for every attempt its call number, whether the operating
system blocked it ("blocked") or it got through ("read"), and whether it was meant to be allowed. The leaked paths of
the other agents' directories and of the logs are given on the command line: a real agent could learn them from `ps`, so the
self-test asks what stops an agent that knows them.

    escape_agent.py --name a --out LOGDIR --clone CLONE --others DIR,DIR --guard DIR,DIR [--origin DIR]
"""
import argparse
import json
import os
import subprocess
import sys


def emit(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


class Escaper:
    def __init__(self, args):
        self.a = args
        self.calls, self.report = 0, []

    def tool(self, name, **inp):
        self.calls += 1
        emit({"type": "assistant", "message": {"id": "m%d" % self.calls, "model": "claude-fake",
                                               "usage": {"input_tokens": 1, "output_tokens": 1},
                                               "content": [{"type": "tool_use", "id": "t%d" % self.calls, "name": name, "input": inp}]}})
        return self.calls

    def attempt(self, label, tool, fn, allowed=False, **inp):
        call = self.tool(tool, **inp)
        try:
            out = fn()
            outcome, detail = "read", str(out)[:80]
        except OSError as e:
            outcome, detail = "blocked", e.__class__.__name__
        except subprocess.CalledProcessError as e:
            outcome, detail = "blocked", "exit %d" % e.returncode
        self.report.append({"label": label, "call": call, "outcome": outcome, "allowed": allowed, "detail": detail})

    def run(self):
        self.allowed_ones()
        self.parent_directories()
        self.guarded_listings()
        self.symlink()
        self.leaked_paths()
        self.finish()

    def allowed_ones(self):
        clone = self.a.clone
        readme = os.path.join(clone, "README.md")
        self.attempt("own file", "Read", lambda: open(readme).read() if os.path.exists(readme) else "", allowed=True, file_path=readme)
        if self.a.origin:
            self.attempt("git log of origin (pushed work only)", "Bash",
                         lambda: subprocess.check_output(["git", "-C", clone, "log", "--oneline", "origin/main"], stderr=subprocess.STDOUT),
                         allowed=True, command="git log --oneline origin/main")

    def parent_directories(self):
        """Names the agent has no way to learn: the agent's own directory is listable, everything above it is not."""
        clone = self.a.clone
        up = lambda *p: os.path.join(clone, "..", *p)
        read = lambda p: open(p).read()
        self.attempt("ls ..", "Bash", lambda: os.listdir(up()), command="python3 -c \"import os; print(os.listdir('..'))\"")
        self.attempt("ls ../..", "Bash", lambda: os.listdir(up("..")), command="python3 -c \"import os; print(os.listdir('../..'))\"")
        self.attempt("cat ../b/SPEC-ROUTING.md", "Bash", lambda: read(up("b", "SPEC-ROUTING.md")), command="cat ../b/SPEC-ROUTING.md")
        self.attempt("python open ../c/x", "Bash", lambda: read(up("c", "x")), command="python3 -c \"open('../c/x').read()\"")
        self.attempt("old layout clones/b", "Read", lambda: read(up("..", "clones", "b", "SPEC.md")), file_path="../../clones/b/SPEC.md")

    def guarded_listings(self):
        for g in [d for d in self.a.guard.split(",") if d][:3]:
            self.attempt("list %s" % os.path.basename(g), "Glob", (lambda g=g: os.listdir(g)), pattern=g + "/*")

    def symlink(self):
        link = os.path.join(self.a.clone, "uplink")
        try:
            os.symlink(os.path.join(self.a.clone, "..", ".."), link)
        except OSError:
            pass
        self.attempt("symlink to the scratch root", "Bash", lambda: os.listdir(link), command="ls ./uplink/")

    def leaked_paths(self):
        """The OS cannot refuse the owner a path it can name, the scan must see it."""
        others = [d for d in self.a.others.split(",") if d]
        read = lambda p: open(p).read()
        if others:
            other = os.path.join(others[0], "clone")
            self.attempt("read another clone by absolute path", "Bash",
                         lambda: [read(os.path.join(other, n)) for n in os.listdir(other) if n.endswith(".md")],
                         command="python3 -c \"print(open('%s/SPEC.md').read())\"" % other)
            self.attempt("git -C another clone", "Bash",
                         lambda: subprocess.check_output(["git", "-C", other, "log", "--oneline"], stderr=subprocess.STDOUT),
                         command="git -C %s log --oneline" % other)
            self.attempt("Read another clone", "Read", lambda: read(os.path.join(other, "README.md")),
                         file_path=os.path.join(other, "README.md"))
        for name in ("a.prompt.md", "a.stream.jsonl"):
            path = os.path.join(self.a.out, name)
            self.attempt("read the log %s by absolute path" % name, "Read", (lambda path=path: read(path)), file_path=path)

    def finish(self):
        with open(os.path.join(self.a.out, "%s.escape.json" % self.a.name), "w") as f:
            json.dump(self.report, f, indent=1)
        emit({"type": "result", "subtype": "success", "is_error": False, "num_turns": self.calls, "total_cost_usd": 0.0,
              "duration_ms": 0, "result": "DONE"})


def main():
    p = argparse.ArgumentParser()
    for k in ("name", "out", "clone"):
        p.add_argument("--" + k, required=True)
    p.add_argument("--others", default="")
    p.add_argument("--guard", default="")
    p.add_argument("--origin", default="")
    args = p.parse_args()
    emit({"type": "system", "subtype": "init", "model": "claude-fake", "session_id": "escape-" + args.name})
    Escaper(args).run()


if __name__ == "__main__":
    main()
