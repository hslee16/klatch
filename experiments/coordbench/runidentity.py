"""The identity of what ran, in the record (#1516): the fixture hash and its version, the hash of the hidden checker, and the hash
of each agent's built prompt. What each covers is in fixhash.py. Split out of run.py (a few lines there call it)."""
import hashlib

import fixhash
from runprompt import build_prompt

# the run's own values in a prompt, as placeholders: two runs of the same arm and task give the same prompt hash
PLACEHOLDERS = {"workdir": "<WORKDIR>", "klatch_bin": "<KLATCH>", "room": "<ROOM>"}


def prompt_hashes(args, fx, arm):
    """{agent: sha256 of its prompt} as build_prompt makes it, or None for an arm whose prompt cannot be built here."""
    minutes = max(1, int(args.wall_s // 60))
    out = {}
    try:
        for a in fx.agents:
            text = build_prompt(args.arms_dir, arm, a["name"], fx.names, a["task"], PLACEHOLDERS["workdir"], fx.test_command,
                                PLACEHOLDERS["klatch_bin"], PLACEHOLDERS["room"], args.max_turns, minutes,
                                restart_command="<RESTART>", topology=fx.topology)
            out[a["name"]] = hashlib.sha256(text.encode()).hexdigest()
    except (OSError, ValueError, KeyError):
        return None
    return out


def identity(args, fx, arm):
    """The record fields that say which fixture, which checker and which prompts a run had."""
    return {"fixture_hash_version": fixhash.FIXTURE_HASH_VERSION,
            "fixture": {"name": fx.name, "sha256": fx.sha256(), "checker_sha256": fx.checker_sha256()},
            "prompt_sha256": prompt_hashes(args, fx, arm)}
