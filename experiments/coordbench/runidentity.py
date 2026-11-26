"""The identity of what ran, in the record (#1516, #1547): the fixture hash and its version, the hash of the hidden checker, the hash
of each agent's built prompt (workers and, in a role arm, the role members) and the hash of the arm text files the arm uses. What
each covers is in fixhash.py. Split out of run.py (a few lines there call it)."""
import hashlib
import os

import fixhash
import roles as roles_mod
import scenario as scenario_mod
from runprompt import build_prompt

# the run's own values in a prompt, as placeholders: two runs of the same arm and task give the same prompt hash
PLACEHOLDERS = {"workdir": "<WORKDIR>", "klatch_bin": "<KLATCH>", "room": "<ROOM>"}


def _role_members(args, fx, arm):
    """The role members of the run as runexec.configure_roles makes them (none for an arm without roles)."""
    if not roles_mod.is_role_arm(arm):
        return []
    scn = None
    if fx.spec.get("scenario"):
        try:
            scn = scenario_mod.load(fx.spec["scenario"], args.scenarios_dir)
        except scenario_mod.ScenarioError:
            scn = None
    return roles_mod.resolve(args, scn, arm, len(fx.names))["members"]


def prompt_hashes(args, fx, arm):
    """{agent: sha256 of its prompt} as build_prompt makes it, plus each role member of a role arm as roles.role_prompt makes it
    (a replacement worker has its predecessor's prompt, so it adds no entry); None for an arm whose prompt cannot be built here."""
    minutes = max(1, int(args.wall_s // 60))
    out = {}
    try:
        for a in fx.agents:
            text = build_prompt(args.arms_dir, arm, a["name"], fx.names, a["task"], PLACEHOLDERS["workdir"], fx.test_command,
                                PLACEHOLDERS["klatch_bin"], PLACEHOLDERS["room"], args.max_turns, minutes,
                                restart_command="<RESTART>", topology=fx.topology)
            out[a["name"]] = hashlib.sha256(text.encode()).hexdigest()
        for member in _role_members(args, fx, arm):
            text = roles_mod.role_prompt(args.arms_dir, arm, member, member, fx.names, PLACEHOLDERS["workdir"], fx.test_command,
                                         PLACEHOLDERS["klatch_bin"], PLACEHOLDERS["room"], "<RESTART>", args.max_turns, minutes)
            out[member] = hashlib.sha256(text.encode()).hexdigest()
    except (OSError, ValueError, KeyError):
        return None
    return out


def arm_text_sha256(arms_dir, arm, topology):
    """sha256 over the arm text files a run of this arm and topology reads (the common part, the base arm and, for a role arm, the
    role file), each with its name; None when one is missing. Other arms' files and the notes in arms/ are not in it."""
    sfx = "-shared" if topology == "T-shared" else ""
    names = ["common%s.md" % sfx, "%s%s.md" % (roles_mod.base_arm(arm), sfx)]
    if roles_mod.is_role_arm(arm):
        names.append(arm + ".md")
    h = hashlib.sha256()
    try:
        for n in names:
            with open(os.path.join(arms_dir, n), "rb") as f:
                h.update(n.encode() + b"\0" + f.read() + b"\0")
    except OSError:
        return None
    return h.hexdigest()


def identity(args, fx, arm):
    """The record fields that say which fixture, which checker, which prompts and which arm texts a run had."""
    return {"fixture_hash_version": fixhash.FIXTURE_HASH_VERSION,
            "fixture": {"name": fx.name, "sha256": fx.sha256(), "checker_sha256": fx.checker_sha256()},
            "prompt_sha256": prompt_hashes(args, fx, arm),
            "arm_text_sha256": arm_text_sha256(args.arms_dir, arm, fx.topology)}
