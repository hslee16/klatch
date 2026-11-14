#!/usr/bin/env python3
"""A launcher starts a headless Claude Code worker that is a verified klatch member from its first action.

    KLATCH_BIN=/path/to/klatch python3 clients/recipes/claude-code-worker.py

The launcher issues a credential and a key bundle (no coding-agent session above it), starts `claude -p` with only the
bundle's path in KLATCH_IDENTITY_FILE (no name, no key directory), asks it to post one message through its Bash tool, and
shows that another member sees the post as verified. It calls a model, so it is not part of CI; it was run on
2026-11-14 with Claude Code 2.1.325 (RESEARCH-NOTES section 40). Needs a signed-in `claude`.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.environ.get("KLATCH_BIN") or os.path.join(HERE, "..", "..", "proto", "target", "debug", "klatch")
BIN = os.path.abspath(BIN)

home = tempfile.mkdtemp(prefix="klatch-worker-")
work = tempfile.mkdtemp(prefix="klatch-work-")
fake_home = tempfile.mkdtemp(prefix="klatch-fakehome-")
keys = os.path.join(home, "keys")
bundle = os.path.join(home, "w-live.bundle")
try:
    # the launcher: a home with no harness registry, so no coding-agent session is found above it
    launcher_env = {"PATH": os.environ["PATH"], "HOME": fake_home, "KLATCH_HOME": home, "KLATCH_KEY_DIR": keys}
    r = subprocess.run(
        [BIN, "worker", "issue", "w-live", "--clearance", "internal", "--ttl", "600", "--parent", "lead", "--out", bundle],
        env=launcher_env, capture_output=True, text=True,
    )
    if r.returncode:
        sys.exit(f"issue failed: {r.stderr}")
    # the worker's hooks join the lobby as the bundle's member; its Bash tool may run klatch and nothing else
    settings = {
        "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": f"{BIN} --discover hook --event SessionStart"}]}]},
    }
    settings_path = os.path.join(work, "settings.json")
    json.dump(settings, open(settings_path, "w"))
    env = {k: v for k, v in os.environ.items() if not k.startswith(("CLAUDE", "KLATCH"))}
    env.update(KLATCH_HOME=home, KLATCH_IDENTITY_FILE=bundle)  # only the path of the bundle
    prompt = f"Run exactly this one shell command and then say done: {BIN} post lobby 'hello from the credentialed worker'"
    p = subprocess.run(
        ["claude", "-p", prompt, "--model", "haiku", "--settings", settings_path, "--setting-sources", "",
         "--allowedTools", f"Bash({BIN}*)"],
        env=env, cwd=work, capture_output=True, text=True, timeout=180, stdin=subprocess.DEVNULL,
    )
    print("claude:", p.stdout.strip()[:200].replace("\n", " "))
    seen = subprocess.run(
        [BIN, "read", "lobby", "--peek"],
        env=dict(launcher_env, KLATCH_NAME="observer"), capture_output=True, text=True,
    ).stdout
    print("an observer reads:", seen.strip())
    if "w-live" not in seen or "(unverified)" in seen:
        sys.exit("the worker's message was not shown as verified")
    print("PASS: the worker was a verified member from its first action")
finally:
    for d in (home, work, fake_home):
        shutil.rmtree(d, ignore_errors=True)
