#!/usr/bin/env python3
"""Receiver and digest for the webhook-driven GitHub watcher (#1329). Python 3 standard library only; run it with
`python3 -I` (the launcher proto/scripts/gh-watch does).

It listens on loopback for events that `gh webhook forward` delivers, verifies each one, turns it into at most a few
one-line notes (INFO or ALERT) and appends them to a log file (and, optionally, posts them to a klatch room).

Read-only by construction: it makes no call to GitHub, runs no other program than the one optional klatch post, and
treats every character of every payload as data (never run, never built into a command, control characters removed,
fields truncated). A test (test_gh_watch.py) asserts the absence of any GitHub call or HTTP client in this file.
"""
import argparse
import collections
import datetime
import hashlib
import hmac
import ipaddress
import json
import os
import re
import stat
import signal
import subprocess
import sys
import threading
import time
import unicodedata
from http.server import BaseHTTPRequestHandler, HTTPServer

# every event the receiver accepts; anything else is answered 200 "ignored" and dropped
ALLOWED_EVENTS = ("pull_request", "check_run", "workflow_run", "push", "issues", "issue_comment", "ping")
FAIL_CONCLUSIONS = ("failure", "timed_out", "startup_failure")
DEFAULT_MAX_BODY = 1024 * 1024
FIELD_MAX = 80
LINE_MAX = 300
SEEN_MAX = 4096
SIGNATURE_PREFIX = "sha256="
# The watcher claim (#1338): the receiver holds this named claim while it is up, so any session sees the watcher in
# `klatch claims` and the board, and a lapse (a kill -9, a reboot) is the dead-watcher signal `klatch stuck` reports.
# Renewed every CLAIM_RENEW seconds with a lease of CLAIM_TTL: three intervals, so two renewals can be missed (a slow
# klatch call, a busy store) before the claim lapses, and a dead receiver shows within CLAIM_TTL seconds.
CLAIM_NAME = "watch:gh"
CLAIM_TTL = 90
CLAIM_RENEW = 30
CLAIM_REASON = "gh-watch receiver"


def clean(value, limit=FIELD_MAX):
    """Payload text as inert, single-line, bounded text: control, format, separator and unassigned characters become
    spaces, runs of white space collapse, and the result is cut at `limit` characters with `...`."""
    if isinstance(value, bool) or not isinstance(value, (str, int)):
        return ""
    text = "".join(" " if unicodedata.category(c) in ("Cc", "Cf", "Zl", "Zp", "Cs", "Co", "Cn") else c
                   for c in str(value))
    text = " ".join(text.split())
    return text if len(text) <= limit else text[: limit - 3] + "..."


def get(obj, *path):
    for key in path:
        if not isinstance(obj, dict):
            return None
        obj = obj.get(key)
    return obj


def pr_numbers(items):
    out = []
    if isinstance(items, list):
        for item in items:
            n = item.get("number") if isinstance(item, dict) else None
            if isinstance(n, int) and not isinstance(n, bool):
                out.append(n)
    return out


class State:
    """The little the digest remembers: which PR numbers were last seen as drafts (a failed check on a draft is not an
    alert). A PR never seen is treated as not a draft."""

    def __init__(self):
        self.drafts = collections.OrderedDict()

    def set_draft(self, number, draft):
        if draft:
            self.drafts[number] = True
            while len(self.drafts) > SEEN_MAX:
                self.drafts.popitem(last=False)
        else:
            self.drafts.pop(number, None)

    def is_draft(self, number):
        return number in self.drafts


def default_branch(payload):
    b = get(payload, "repository", "default_branch")
    return b if isinstance(b, str) and b else "main"


def short(sha):
    return clean(sha, 40)[:7] if isinstance(sha, str) else "?"


def digest(event, payload, state):
    """One event to a list of (severity, text). Never raises on a malformed payload: it returns what it can read."""
    if not isinstance(payload, dict):
        return []
    try:
        return _digest(event, payload, state)
    except (KeyError, TypeError, ValueError, AttributeError):
        return []


def _digest(event, payload, state):
    action = payload.get("action")
    main = default_branch(payload)
    out = []
    if event == "pull_request":
        p = payload.get("pull_request")
        n = payload.get("number", get(p, "number"))
        if not isinstance(p, dict) or not isinstance(n, int):
            return []
        state.set_draft(n, bool(p.get("draft")) and p.get("state") != "closed")
        who = clean(get(p, "user", "login"))
        title = clean(p.get("title"))
        branch = clean(get(p, "head", "ref"))
        base = clean(get(p, "base", "ref")) or main
        tail = "by %s: %s (%s)" % (who, title, branch)
        if action == "opened":
            out.append(("INFO", "PR #%d opened%s %s" % (n, " (draft)" if p.get("draft") else "", tail)))
        elif action == "reopened":
            out.append(("INFO", "PR #%d reopened %s" % (n, tail)))
        elif action == "ready_for_review":
            out.append(("INFO", "PR #%d ready for review %s" % (n, tail)))
        elif action == "synchronize":
            out.append(("INFO", "PR #%d updated (new push) %s" % (n, tail)))
        elif action == "closed":
            word = "merged" if p.get("merged") is True else "closed"
            out.append(("INFO", "PR #%d %s %s" % (n, word, tail)))
        if p.get("state") != "closed" and action != "closed" and p.get("mergeable_state") == "dirty":
            out.append(("ALERT", "PR #%d conflicts with %s (GitHub mergeable_state dirty): %s" % (n, base, title)))
    elif event in ("check_run", "workflow_run"):
        obj = payload.get(event)
        if not isinstance(obj, dict) or action != "completed" or obj.get("conclusion") not in FAIL_CONCLUSIONS:
            return []
        name = clean(obj.get("name"))
        what = "check" if event == "check_run" else "workflow"
        branch = get(obj, "check_suite", "head_branch") if event == "check_run" else obj.get("head_branch")
        concl = clean(obj.get("conclusion"))
        if branch == main:
            out.append(("ALERT", "%s %s %s on %s at %s: main is red" % (what, name, concl, clean(main),
                                                                         short(obj.get("head_sha")))))
        else:
            for n in pr_numbers(obj.get("pull_requests")):
                if not state.is_draft(n):
                    out.append(("ALERT", "%s %s %s on PR #%d (%s)" % (what, name, concl, n, clean(branch))))
    elif event == "push":
        if payload.get("ref") == "refs/heads/" + main and payload.get("deleted") is not True:
            commits = payload.get("commits")
            count = len(commits) if isinstance(commits, list) else 0
            out.append(("INFO", "%s moved to %s (%d commits, pushed by %s); open PRs may now conflict" % (
                clean(main), short(payload.get("after")), count, clean(get(payload, "pusher", "name")))))
    elif event == "issues":
        i = payload.get("issue")
        n = get(i, "number")
        if not isinstance(n, int):
            return []
        title = clean(get(i, "title"))
        if action == "opened":
            out.append(("INFO", "issue #%d opened by %s: %s" % (n, clean(get(i, "user", "login")), title)))
        elif action == "labeled":
            out.append(("INFO", "issue #%d labeled %s: %s" % (n, clean(get(payload, "label", "name")), title)))
    # issue_comment and ping are accepted (so the sender sees success) but produce no note: comment text is the
    # least trustworthy input and awareness of it is not part of this watcher
    return [(sev, text[:LINE_MAX]) for sev, text in out]


def check_bind(addr):
    try:
        ip = ipaddress.ip_address(addr)
    except ValueError:
        return None
    return ip if ip.is_loopback else None


def check_secret_file(path):
    """The secret bytes, or an error string. The file must be a regular file, owned by this user, mode 0600 (no group
    or other access) and not empty."""
    try:
        st = os.stat(path)
    except OSError as e:
        return None, "cannot read the secret file: %s" % e.strerror
    if not stat.S_ISREG(st.st_mode):
        return None, "the secret file is not a regular file"
    if st.st_mode & 0o077:
        return None, "the secret file must be mode 0600 (no group or world access); run: chmod 600 FILE"
    if st.st_uid != os.getuid():
        return None, "the secret file is not owned by this user"
    with open(path, "rb") as f:
        secret = f.read().strip()
    if not secret:
        return None, "the secret file is empty"
    return secret, None


class Sink:
    def __init__(self, out_dir, klatch_bin, member, room, post_min):
        os.makedirs(out_dir, mode=0o700, exist_ok=True)
        self.fd = os.open(os.path.join(out_dir, "gh-watch.log"), os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
        self.klatch_bin, self.member, self.room, self.post_min = klatch_bin, member, room, post_min

    def emit(self, severity, text):
        now = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
        # one unbuffered write per line, so `tail -F` sees it at once and a crash never leaves half a line
        os.write(self.fd, ("%s %s %s\n" % (now, severity, text)).encode("utf-8", "replace"))
        if self.klatch_bin and (severity == "ALERT" or self.post_min == "INFO"):
            self.post("%s %s" % (severity, text))

    def klatch(self, args):
        """The only program this file runs: `klatch ARGS` as the member, with the caller's KLATCH_HOME. Returns
        (returncode, stderr text); (-1, ...) when it could not run at all."""
        env = dict(os.environ)
        env["KLATCH_NAME"] = self.member
        try:
            r = subprocess.run([self.klatch_bin] + list(args), env=env, timeout=15, check=False,
                               stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        except (OSError, subprocess.SubprocessError) as e:
            sys.stderr.write("gh-watch: klatch %s failed: %s\n" % (args[0], type(e).__name__))
            sys.stderr.flush()
            return -1, ""
        return r.returncode, r.stderr.decode("utf-8", "replace")

    def post(self, line):
        self.klatch(["post", self.room, line])


class ClaimKeeper:
    """Holds the named watcher claim (#1338). `run(args)` runs klatch and returns (returncode, stderr); `clock()` is
    seconds (monotonic in the receiver, fake in the tests). A refusal is told by the fixed prefix `refused:` that klatch
    puts on a claim conflict; nothing else in the text is read or trusted, and the text is never printed raw."""

    def __init__(self, run, clock, room, name=CLAIM_NAME, ttl=CLAIM_TTL, interval=CLAIM_RENEW):
        self.run, self.clock, self.room, self.name, self.ttl, self.interval = run, clock, room, name, ttl, interval
        self.held = False
        self.due = clock() + interval
        self.lock = threading.Lock()

    def _claim(self):
        rc, err = self.run(["claim", self.room, self.name, "--ttl", str(self.ttl), "--shared", "--reason", CLAIM_REASON])
        if rc == 0:
            return "held"
        return "refused" if err.startswith("refused:") else "failed"

    def acquire(self):
        """Take the claim: held, refused (another holder; this receiver must not run) or failed (klatch trouble)."""
        with self.lock:
            state = self._claim()
            self.held = state == "held"
            self.due = self.clock() + self.interval
            return state

    def tick(self):
        """Renew when the interval has passed. A stall renews once, not in a burst. True when it tried."""
        with self.lock:
            now = self.clock()
            if now < self.due:
                return False
            was = self.held
            self.held = self._claim() == "held"
            self.due = now + self.interval
            if was and not self.held:
                sys.stderr.write("gh-watch: the %s claim is not held (renewal failed or another holder took it)\n"
                                 % self.name)
                sys.stderr.flush()
            return True

    def release(self):
        """Give the claim up on a clean stop, once; never when it is not ours."""
        with self.lock:
            if not self.held:
                return
            self.held = False
            self.run(["release", self.room, self.name])


class Handler(BaseHTTPRequestHandler):
    server_version = "gh-watch"
    sys_version = ""
    timeout = 10

    def log_message(self, fmt, *args):  # no per-request access log (it could carry payload-derived text)
        pass

    def reply(self, code, text):
        body = (text + "\n").encode()
        self.send_response(code)
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)
        self.close_connection = True

    def do_GET(self):
        self.reply(405, "method not allowed")

    do_PUT = do_DELETE = do_PATCH = do_HEAD = do_OPTIONS = do_GET

    def do_POST(self):
        srv = self.server
        if self.path != "/hook":
            return self.reply(404, "not found")
        try:
            length = int(self.headers.get("Content-Length", ""))
        except ValueError:
            return self.reply(411, "length required")
        if length < 0:
            return self.reply(400, "bad length")
        if length > srv.max_body:
            return self.reply(413, "body too large")
        body = self.rfile.read(length)
        sent = self.headers.get("X-Hub-Signature-256", "")
        want = SIGNATURE_PREFIX + hmac.new(srv.secret, body, hashlib.sha256).hexdigest()
        if not sent.startswith(SIGNATURE_PREFIX) or not hmac.compare_digest(sent.encode(), want.encode()):
            return self.reply(401, "bad signature")
        event = self.headers.get("X-GitHub-Event", "")
        delivery = self.headers.get("X-GitHub-Delivery", "")
        if not re.fullmatch(r"[A-Za-z0-9-]{1,64}", delivery) or not re.fullmatch(r"[a-z_]{1,40}", event):
            return self.reply(400, "bad headers")
        if event not in ALLOWED_EVENTS:
            return self.reply(200, "ignored")
        if delivery in srv.seen:
            return self.reply(200, "duplicate")
        try:
            payload = json.loads(body.decode("utf-8"))
        except (UnicodeDecodeError, ValueError):
            return self.reply(400, "bad json")
        srv.seen[delivery] = True
        while len(srv.seen) > SEEN_MAX:
            srv.seen.popitem(last=False)
        for severity, text in digest(event, payload, srv.state):
            srv.sink.emit(severity, text)
        self.reply(200, "ok")


class Server(HTTPServer):
    def __init__(self, addr, family, secret, sink, max_body):
        self.address_family = family
        super().__init__(addr, Handler)
        self.secret, self.sink, self.max_body = secret, sink, max_body
        self.seen = collections.OrderedDict()
        self.state = State()


def main(argv):
    ap = argparse.ArgumentParser(prog="gh-watch-receiver", description="Loopback receiver and digest for forwarded "
                                 "GitHub webhooks (read-only; see PR-SWEEP.md).")
    ap.add_argument("--secret-file", required=True, help="file holding the webhook secret, mode 0600")
    ap.add_argument("--out-dir", required=True, help="directory for gh-watch.log (created mode 0700)")
    ap.add_argument("--bind", default="127.0.0.1", help="loopback address only (default 127.0.0.1)")
    ap.add_argument("--port", type=int, default=8787, help="port (0 picks a free one; default 8787)")
    ap.add_argument("--max-body", type=int, default=DEFAULT_MAX_BODY, help="largest accepted body in bytes")
    ap.add_argument("--klatch-bin", help="installed klatch binary used to post each note (optional)")
    ap.add_argument("--as-member", help="member name the posts are made as (needs --klatch-bin)")
    ap.add_argument("--room", help="room the posts go to (needs --klatch-bin)")
    ap.add_argument("--no-claim", action="store_true", help="do not hold the %s claim (needs --klatch-bin to be on)"
                    % CLAIM_NAME)
    ap.add_argument("--claim-ttl", type=int, default=CLAIM_TTL, help="seconds the claim lasts (default %d)" % CLAIM_TTL)
    ap.add_argument("--claim-renew", type=int, default=CLAIM_RENEW,
                    help="seconds between renewals, shorter than the ttl (default %d)" % CLAIM_RENEW)
    ap.add_argument("--post-min", choices=("INFO", "ALERT"), default="INFO", help="lowest severity posted")
    a = ap.parse_args(argv)

    ip = check_bind(a.bind)
    if ip is None:
        sys.stderr.write("gh-watch: refusing to bind %s: only loopback addresses are allowed (see #157/#161)\n"
                         % clean(a.bind))
        return 2
    if "/.klatch" in os.path.abspath(a.out_dir) + "/":
        sys.stderr.write("gh-watch: refusing an --out-dir inside .klatch (never the live store)\n")
        return 2
    if a.klatch_bin:
        ident = re.compile(r"[A-Za-z0-9][A-Za-z0-9._:-]*")
        if not (a.as_member and a.room and ident.fullmatch(a.as_member) and ident.fullmatch(a.room)):
            sys.stderr.write("gh-watch: --klatch-bin needs --as-member and --room (plain names)\n")
            return 2
    elif a.as_member or a.room:
        sys.stderr.write("gh-watch: --as-member and --room need --klatch-bin\n")
        return 2
    if not 1 <= a.max_body <= 25 * 1024 * 1024:
        sys.stderr.write("gh-watch: --max-body must be between 1 and 26214400\n")
        return 2
    if not 1 <= a.claim_renew < a.claim_ttl <= 300:
        sys.stderr.write("gh-watch: need 1 <= --claim-renew < --claim-ttl <= 300\n")
        return 2
    secret, err = check_secret_file(a.secret_file)
    if err:
        sys.stderr.write("gh-watch: %s\n" % err)
        return 2
    import socket
    family = socket.AF_INET6 if ip.version == 6 else socket.AF_INET
    sink = Sink(a.out_dir, a.klatch_bin, a.as_member, a.room, a.post_min)
    try:
        server = Server((a.bind, a.port), family, secret, sink, a.max_body)
    except OSError as e:
        sys.stderr.write("gh-watch: cannot listen: %s\n" % e.strerror)
        return 2
    keeper = None
    if a.klatch_bin and not a.no_claim:
        keeper = ClaimKeeper(sink.klatch, time.monotonic, a.room, ttl=a.claim_ttl, interval=a.claim_renew)
        if keeper.acquire() == "refused":
            sys.stderr.write("gh-watch: another receiver holds the %s claim in %s; not starting a second one\n"
                             % (CLAIM_NAME, a.room))
            server.server_close()
            return 2
    stop = threading.Event()

    def renew_loop():
        while not stop.wait(1.0):
            keeper.tick()

    if keeper:
        threading.Thread(target=renew_loop, daemon=True).start()

    def on_term(signum, frame):
        # shutdown() waits for serve_forever, so it cannot run in this handler (the main thread is in serve_forever)
        threading.Thread(target=server.shutdown, daemon=True).start()

    signal.signal(signal.SIGTERM, on_term)
    signal.signal(signal.SIGHUP, on_term)
    sys.stdout.write("listening %s:%d\n" % (a.bind, server.server_address[1]))
    sys.stdout.flush()
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        stop.set()
        if keeper:
            keeper.release()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
