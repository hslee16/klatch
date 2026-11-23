"""Role members for the arms K3 and B1-roles (#1421): the integrator and the supervisor, started beside the workers.

A role member is its own agent process in the run (a `claude -p` session in the real driver, a scripted fake in tests) with
its own clone, its own spend cap, its own cost line in the run record and, in K3, its own member name in the run store.

  prompts   arms/K3.md and arms/B1-roles.md hold three sections each (worker, integrator, supervisor). The worker prompt is
            the base arm's text (K2, B1) plus the worker section; a role prompt is a short frame, the same words in both arms,
            plus the role's section. The frame names no tool and no klatch word.
  grants    klatch grants roles only to the human at a terminal: `klatch admin integrator MEMBER` asks the admin passphrase at
            a pseudo-terminal. The bench does the same in a run store it made itself (klatch-bench World.admin): the
            harness sets a throwaway passphrase for the RUN store (random, kept in memory, never written, never printed, never
            in a record) and types it at a pseudo-terminal of its own. It is the same code path a person uses, on the run's
            temporary KLATCH_HOME, so the live store, the human's passphrase and the Keychain are never involved. The
            guard in run.py has already refused any store path that could be the live one.
  restart   the supervisor starts a replacement through restart_member.py, which only writes a request file; the harness
            reads it and starts the replacement member (it holds the credentials and the budget, the supervisor does not).

Standard library only (`pty` for the grant); POSIX.
"""
import os
import pty
import re
import secrets
import select
import time

ROLES = ("integrator", "supervisor")
ROLE_ARMS = {"K3": "K2", "B1-roles": "B1"}
DEFAULT_GRACE_S = 120
DEFAULT_MAX_RESTARTS = 1

FRAME = ("You are a member of a team of agents building software together in one git repository. Your working directory is "
         "`{WORKDIR}`, a clone of the shared repository (`origin`, branch `main`). The workers are: {WORKERS}. Nobody will "
         "answer a question you put to a human, so do not ask one. You have at most {MAX_TURNS} turns and about {MAX_MINUTES} "
         "minutes; stop before you run out.\n\n")


def is_role_arm(arm):
    return arm in ROLE_ARMS


def base_arm(arm):
    """The arm a role arm adds roles to (K3: K2, B1-roles: B1); any other arm is its own."""
    return ROLE_ARMS.get(arm, arm)


def strip_comments(text):
    return re.sub(r"<!--.*?-->", "", text, flags=re.S).strip()


def split_sections(text):
    """The '## ' sections of a role arm file keyed worker, integrator or supervisor by the first word of the heading."""
    out = {}
    for part in re.split(r"^## ", strip_comments(text), flags=re.M)[1:]:
        heading, _, body = part.partition("\n")
        out[heading.strip().split()[0].lower()] = body.strip()
    return out


def sections_of(arms_dir, arm):
    with open(os.path.join(arms_dir, arm + ".md")) as f:
        return split_sections(f.read())


def fill(text, values):
    for k, v in values.items():
        text = text.replace("{{%s}}" % k, v)
    left = re.findall(r"\{\{[A-Z_]+\}\}", text)
    if left:
        raise ValueError("unfilled placeholders: %s" % ", ".join(sorted(set(left))))
    return text


def role_prompt(arms_dir, arm, role, name, workers, workdir, test_command, klatch_bin, room, restart_command,
                max_turns, max_minutes):
    """The prompt of one role member: the shared frame and the role's section of the arm file, every placeholder filled."""
    secs = sections_of(arms_dir, arm)
    if role not in secs:
        raise ValueError("%s.md has no %s section" % (arm, role))
    values = {"NAME": name, "WORKDIR": workdir, "TEST_COMMAND": test_command, "KLATCH": klatch_bin or "", "ROOM": room,
              "RESTART_COMMAND": restart_command or ""}
    frame = FRAME.format(WORKDIR=workdir, WORKERS=", ".join(workers), MAX_TURNS=max_turns, MAX_MINUTES=max_minutes)
    return frame + fill(secs[role], values) + "\n"


def worker_text(arms_dir, arm):
    """The worker's section of a role arm file (it follows the base arm's text in the worker prompt)."""
    secs = sections_of(arms_dir, arm)
    if "worker" not in secs:
        raise ValueError("%s.md has no worker section" % arm)
    return secs["worker"]


def resolve(a, scn, arm, n_workers):
    """The role settings of one run, from the command line over the scenario file over the defaults; None for an arm
    without roles. The scenario file's `roles` object: arms, members, model, per_role_usd, grace_s, max_restarts."""
    if arm not in ROLE_ARMS:
        return None
    cfg = (scn or {}).get("roles") or {}
    if cfg.get("arms") is not None and arm not in cfg["arms"]:
        return None
    given = getattr(a, "roles", None)
    members = [m for m in given.split(",") if m] if given else list(cfg.get("members") or ROLES)
    for m in members:
        if m not in ROLES:
            raise ValueError("unknown role %r (the roles are %s)" % (m, ", ".join(ROLES)))
    per = getattr(a, "role_usd", None)
    if per is None:
        per = cfg.get("per_role_usd")
    if per is None:
        per = a.max_run_usd / float(n_workers + len(members))
    grace = getattr(a, "roles_grace_s", None)
    if grace is None:
        grace = cfg.get("grace_s", DEFAULT_GRACE_S)
    return {"members": members, "model": getattr(a, "roles_model", None) or cfg.get("model") or a.model,
            "per_role_usd": float(per), "grace_s": float(grace),
            "max_restarts": int(cfg.get("max_restarts", DEFAULT_MAX_RESTARTS)),
            "clearance": dict(cfg.get("clearance") or {})}


# the admin seam -----------------------------------------------------------------------------------------------


def run_on_pty(argv, typed, env, cwd, timeout=60):
    """Run argv on a pseudo-terminal of its own, which is also its controlling terminal (so /dev/tty opens), and type one
    line from `typed` each time the output ends in a prompt (`: ` or `> `), after the prompt. Returns (exit code, the
    terminal text). The typed lines are not echoed by the terminal (a passphrase prompt turns echo off)."""
    pid, fd = pty.fork()
    if pid == 0:  # the child: its own session, the slave as its terminal
        try:
            os.chdir(cwd)
            os.execve(argv[0], argv, env)
        finally:
            os._exit(127)
    text, nxt, answered = "", 0, 0
    deadline = time.monotonic() + timeout
    try:
        while time.monotonic() < deadline:
            ready, _, _ = select.select([fd], [], [], 0.2)
            if not ready:
                continue
            try:
                chunk = os.read(fd, 4096)
            except OSError:  # the child closed the terminal
                break
            if not chunk:
                break
            text += chunk.decode("utf-8", "replace").replace("\r", "")
            if text.endswith((": ", "> ")) and len(text) > answered and nxt < len(typed):
                os.write(fd, (typed[nxt] + "\n").encode())
                nxt += 1
                answered = len(text)
    finally:
        os.close(fd)
    try:
        _, status = os.waitpid(pid, 0)
    except ChildProcessError:
        return 1, text
    return os.waitstatus_to_exitcode(status), text


class AdminSeam:
    """`klatch admin` on a run store the harness made, the way a person at a terminal does it. The passphrase is random,
    exists only in this object and is never written, printed or recorded; it protects the run's temporary store, which is
    deleted at the end of the run."""

    def __init__(self, klatch_bin, env, cwd):
        self.klatch_bin, self.env, self.cwd = klatch_bin, dict(env, KLATCH_NAME="harness"), cwd
        self._passphrase = secrets.token_urlsafe(24)
        self._ready = False

    def _run(self, args, typed):
        return run_on_pty([self.klatch_bin] + list(args), typed, self.env, self.cwd)

    def act(self, args):
        """(ok, why) of `klatch admin ARGS`; why is empty when it worked, else the last line the terminal showed."""
        if not self._ready:
            code, text = self._run(["admin", "init-passphrase"], [self._passphrase, self._passphrase])
            if code != 0:
                return False, "setting the run store's admin passphrase failed: %s" % _last_line(text)
            self._ready = True
        code, text = self._run(["admin"] + list(args), [self._passphrase])
        return code == 0, "" if code == 0 else "klatch admin %s failed: %s" % (" ".join(args), _last_line(text))


def _last_line(text):
    lines = [l for l in text.splitlines() if l.strip() and not l.startswith("admin passphrase to approve")]
    return lines[-1][:200] if lines else "no output"


def grant_integrator(seam, member):
    """Give `member` the integrator role in the run store: (ok, why)."""
    return seam.act(["integrator", member])


def set_clearance(seam, member, label):
    """Set a member's clearance in the run store, at most the agent cap: (ok, why)."""
    return seam.act(["clear", member, label])
