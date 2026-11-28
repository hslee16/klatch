#!/usr/bin/env python3
"""Run `klatch admin ...` on a pseudo-terminal in a throwaway KLATCH_HOME, typing the admin passphrase at its prompts.

Test support for the client suites (#1773); nothing here is used by the clients. The product takes the admin
passphrase only from a terminal, so a test that needs an admin act (make a reviewer, label a room, clear a member)
drives it the way the Rust helpers do (`proto/klatch/tests/common/pty.rs`): the command gets the terminal as its
standard input, output and error and as its controlling terminal, and a line is typed each time the output ends in a
prompt (`: ` or `> `) that is new since the last line typed. The passphrase is set once per home (typed twice at
`admin init-passphrase`) and a marker file records that. Standard library only, Unix only.

    klatch_admin.py KLATCH_BIN HOME ADMIN_ARGS...      e.g.  klatch_admin.py ./klatch /tmp/h reviewer rev

Prints the terminal text and exits with the command's status. It refuses a HOME that is not under the system temp
directory, so it can never touch the live store, the user's key directory or `~/.klatch`.
"""
import os
import pty
import select
import sys
import tempfile
import time

PASS = "correct horse battery staple"
DEADLINE = 120  # seconds, for one command


def is_prompt(text):
    last = text.rsplit("\n", 1)[-1]
    return (text.endswith(": ") or text.endswith("> ")) and not last.startswith("Error:")


def run_on_pty(argv, env, typed):
    """Run argv on a pty, typing one line per new prompt. Returns (exit status, terminal text)."""
    pid, master = pty.fork()
    if pid == 0:
        os.execve(argv[0], argv, env)
    text, next_line, answered_at = "", 0, 0
    end = time.monotonic() + DEADLINE
    while True:
        if time.monotonic() > end:
            os.kill(pid, 9)
            raise RuntimeError("the command did not finish on its pty; so far:\n" + text)
        ready, _, _ = select.select([master], [], [], 0.1)
        if ready:
            try:
                got = os.read(master, 4096)
            except OSError:
                break
            if not got:
                break
            text += got.decode("utf-8", "replace").replace("\r", "")
            if is_prompt(text) and len(text) > answered_at and next_line < len(typed):
                os.write(master, (typed[next_line] + "\n").encode())
                next_line += 1
                answered_at = len(text)
        else:
            done, status = os.waitpid(pid, os.WNOHANG)
            if done:
                # the child ended: read what is left, then stop
                try:
                    while select.select([master], [], [], 0)[0]:
                        got = os.read(master, 4096)
                        if not got:
                            break
                        text += got.decode("utf-8", "replace").replace("\r", "")
                except OSError:
                    pass
                return os.waitstatus_to_exitcode(status), text
    _, status = os.waitpid(pid, 0)
    return os.waitstatus_to_exitcode(status), text


def admin(klatch_bin, home, args):
    """Run `klatch admin ARGS` in `home`, setting the passphrase first if this home has none. Returns (status, text)."""
    home = os.path.realpath(home)
    tmp = os.path.realpath(tempfile.gettempdir())
    if not home.startswith(tmp + os.sep) and not home.startswith("/tmp" + os.sep) and not home.startswith("/private/tmp" + os.sep):
        raise SystemExit(f"refusing a home outside the temp directory: {home}")
    fake = os.path.join(home, "fake-home")
    os.makedirs(fake, exist_ok=True)
    env = {k: v for k, v in os.environ.items() if k not in ("KLATCH_NAME", "KLATCH_IDENTITY_FILE", "KLATCH_DISCOVER")}
    env.update(HOME=fake, KLATCH_HOME=home, KLATCH_KEY_DIR=os.path.join(home, "keys"))
    marker = os.path.join(home, "passphrase-was-set")
    if not os.path.exists(marker):
        status, text = run_on_pty([klatch_bin, "admin", "init-passphrase"], env, [PASS, PASS])
        if status != 0:
            return status, text
        open(marker, "w").close()
    return run_on_pty([klatch_bin, "admin", *args], env, [PASS])


if __name__ == "__main__":
    if len(sys.argv) < 4:
        raise SystemExit(__doc__)
    code, out = admin(sys.argv[1], sys.argv[2], sys.argv[3:])
    sys.stdout.write(out)
    sys.exit(code)
