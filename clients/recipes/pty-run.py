"""Run a command on a pseudo-terminal and type the demo's admin passphrase when it asks (#1233).

klatch asks for the admin passphrase at a terminal and never reads it from the environment, an argument or a pipe,
so a demo script that plays the launcher gives each launcher command a terminal of its own and types for it. The
passphrase here is the demo's own, for a throwaway store. A real launcher is a person at a terminal.

    python3 -I pty-run.py LINE [LINE ...] -- COMMAND [ARG ...]   (each LINE is typed at one prompt, in order)
"""
import fcntl
import os
import select
import subprocess
import sys
import termios

cut = sys.argv.index("--")
typed, cmd = sys.argv[1:cut], sys.argv[cut + 1 :]
master, slave = os.openpty()


def own_terminal():
    os.setsid()
    fcntl.ioctl(0, termios.TIOCSCTTY, 0)


proc = subprocess.Popen(cmd, stdin=slave, stdout=slave, stderr=slave, preexec_fn=own_terminal)
os.close(slave)
text, sent, answered = "", 0, 0
while True:
    ready, _, _ = select.select([master], [], [], 0.2)
    if ready:
        try:
            data = os.read(master, 4096)
        except OSError:
            break
        if not data:
            break
        text += data.decode(errors="replace")
        prompting = text.endswith(": ") or text.endswith("> ")
        if prompting and len(text) > answered and sent < len(typed):
            os.write(master, (typed[sent] + "\n").encode())
            sent += 1
            answered = len(text)
    elif proc.poll() is not None:
        break
code = proc.wait()
sys.stdout.write("".join(l for l in text.replace("\r", "").splitlines(True) if not l.startswith("admin passphrase to approve")))
sys.exit(code)
