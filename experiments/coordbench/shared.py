"""The shared working tree topology T-shared (#1427): helpers for run.py.

All agents of a run start in ONE directory (a clone of the run's bare origin). There is no per-agent clone, so nothing
that git history alone could say about who lost whose edit survives: two agents that both save a file leave only the
last save. This module supplies what replaces it:

  Snapshots / Watcher  the shared file as it is on disk, read by a light thread every --snapshot-s seconds (default 5) and
                       once whenever an agent exits, and again at the very end. Only a version that differs from the previous
                       one is kept. A ticket whose behaviour (the hidden checker's --file mode) is present in some snapshot
                       and absent from the final file was lost.
  lost_edit_events     from the agents' own streams: every Write, Edit or MultiEdit tool call on the file whose added lines
                       (lines not in the initial file) are not all in the final file. It sees an edit that no snapshot caught.
  finalise             the final state: when the tree has uncommitted changes the harness commits them under the neutral
                       author FINAL_AUTHOR; the checker then runs on a clone of the tree's HEAD.

Resolution limit, stated plainly: an edit that was written and overwritten between two snapshots, by an agent that is still
running, leaves no snapshot behind. The stream events still show it (lost_edit_events), but only as "lines this agent added
are gone", which also counts an agent that deliberately removed its own earlier line, and is blind to a change made with a
shell command (sed, a script) rather than a file tool. Neither measure sees a write by an agent that was never seen to run
a tool. The record carries the interval, the number of snapshots and this text.
"""
import json
import os
import subprocess
import threading
import time
import streamshape

FINAL_AUTHOR = "harness-final"
LIMIT = ("snapshot resolution: an edit written and overwritten between two snapshots (the interval, plus a snapshot whenever an "
         "agent exits) is invisible to the snapshot measure; the stream measure (lost_edit_events) catches Write, Edit and "
         "MultiEdit calls only, counts an agent's own deliberate removal of its earlier line as lost, and cannot see a change "
         "made by a shell command. overwrites is the union, each entry says which measure found it (source).")
EDIT_TOOLS = ("Write", "Edit", "MultiEdit")


def git_out(tree, args, env):
    p = subprocess.run(["git"] + args, cwd=tree, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    return p.returncode, p.stdout


def read_text(path):
    try:
        with open(path, errors="replace") as f:
            return f.read()
    except OSError:
        return None


class Snapshots:
    """The kept versions of one file. add() keeps a version only when it differs from the last kept one."""

    def __init__(self):
        self.distinct = []  # [{"t", "reason", "text"}]
        self.taken = 0
        self._lock = threading.Lock()

    def add(self, t, reason, text):
        with self._lock:
            self.taken += 1
            if text is None or (self.distinct and self.distinct[-1]["text"] == text):
                return False
            self.distinct.append({"t": round(t, 3), "reason": reason, "text": text})
            return True

    def texts(self):
        with self._lock:
            return [d["text"] for d in self.distinct]

    def summary(self, interval_s):
        with self._lock:
            return {"interval_s": interval_s, "taken": [{"t": d["t"], "reason": d["reason"]} for d in self.distinct],
                    "count": self.taken, "distinct": len(self.distinct), "limit": LIMIT}


class Watcher(threading.Thread):
    """Reads `path` every interval_s seconds and whenever one of the processes in procs() has exited since the last look."""

    def __init__(self, path, interval_s, procs, t0, snaps):
        super().__init__(daemon=True)
        self.path, self.interval_s, self.procs, self.t0, self.snaps = path, interval_s, procs, t0, snaps
        self._halt = threading.Event()
        self._gone = set()

    def take(self, reason):
        self.snaps.add(time.monotonic() - self.t0, reason, read_text(self.path))

    def run(self):
        last = time.monotonic()
        self.take("start")
        while not self._halt.wait(0.02):
            for key, proc in list(self.procs().items()):
                if key not in self._gone and proc.poll() is not None:
                    self._gone.add(key)
                    self.take("exit:%s" % key)
            if time.monotonic() - last >= self.interval_s:
                last = time.monotonic()
                self.take("tick")

    def finish(self):
        self._halt.set()
        self.join(timeout=5)
        self.take("end")


def _added_lines(new_text, base_lines):
    return [l.strip() for l in (new_text or "").splitlines() if l.strip() and l.strip() not in base_lines]


def lost_edit_events(stream_lines, rel_path, base_text, final_text):
    """The edit tool calls, per agent, on the shared file whose added lines are not all in the final file.

    stream_lines: {agent: [stream-json lines]}. A Write adds the lines of its content that the initial file did not have; an
    Edit (or each edit of a MultiEdit) the lines of new_string (or of an `append`) that were not in the initial file. Returns
    [{"agent", "tool", "index" (the event's position among that agent's edits of the file), "added_lines", "lost_lines"}],
    only for events that lost something."""
    base_lines = {l.strip() for l in (base_text or "").splitlines()}
    final_lines = {l.strip() for l in (final_text or "").splitlines()}
    out = []
    for agent in sorted(stream_lines):
        index = 0
        for ln in stream_lines[agent]:
            try:
                msg = json.loads(ln)
            except ValueError:
                continue
            if not isinstance(msg, dict) or msg.get("type") != "assistant":
                continue
            for block in streamshape.content(msg):
                if not isinstance(block, dict) or block.get("type") != "tool_use" or block.get("name") not in EDIT_TOOLS:
                    continue
                inp = block.get("input") or {}
                target = inp.get("file_path") or ""
                if os.path.normpath(target) != os.path.normpath(rel_path) and not target.endswith("/" + rel_path):
                    continue
                if block["name"] == "Write":
                    texts = [inp.get("content")]
                elif block["name"] == "MultiEdit":
                    texts = [e.get("new_string") for e in inp.get("edits") or [] if isinstance(e, dict)]
                else:
                    texts = [inp.get("new_string", inp.get("append"))]
                added = [l for t in texts for l in _added_lines(t, base_lines)]
                lost = [l for l in added if l not in final_lines]
                index += 1
                if lost:
                    out.append({"agent": agent, "tool": block["name"], "index": index, "added_lines": len(added),
                                "lost_lines": len(lost)})
    return out


def finalise(tree, env):
    """Commit what is uncommitted in the shared tree under FINAL_AUTHOR. Returns {"made": bool, "author", "files": [...]}."""
    git_out(tree, ["add", "-A"], env)
    rc, status = git_out(tree, ["status", "--porcelain"], env)
    files = [l[3:] for l in status.splitlines() if l.strip()]
    if rc != 0 or not files:
        return {"made": False, "author": None, "files": []}
    e = dict(env, GIT_AUTHOR_NAME=FINAL_AUTHOR, GIT_COMMITTER_NAME=FINAL_AUTHOR,
             GIT_AUTHOR_EMAIL=FINAL_AUTHOR + "@bench.invalid", GIT_COMMITTER_EMAIL=FINAL_AUTHOR + "@bench.invalid")
    rc, out = git_out(tree, ["commit", "-q", "-m", "harness: commit the shared working tree at the end of the run"], e)
    if rc != 0:
        raise RuntimeError("the final commit of the shared tree failed: %s" % out.strip()[-200:])
    return {"made": True, "author": FINAL_AUTHOR, "files": files}
