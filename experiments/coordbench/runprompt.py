"""The agent prompt of a run (split out of run.py, #1455): the common part with the arm's coordination section, every
placeholder filled, draft comments removed. Standard library only."""
import os
import re

import roles as roles_mod

TASK_BOARD = ("# Task board\n\nOne line per piece of work: `- WHO | STATUS | FILES | TITLE`. STATUS is `working`, `done`, "
              "`blocked` or `handoff`. Add your line at the end, commit it and push it.\n")

# git merges two appends to the board without a conflict (#1523); the harness commits it with the board in every B1 repository
BOARD_ATTRIBUTES = "TASKS.md merge=union\n"


def _load(arms_dir, fn):
    """The arm file with its draft comments removed."""
    with open(os.path.join(arms_dir, fn)) as f:
        text = re.sub(r"<!--.*?-->", "", f.read(), flags=re.S)
    return text.strip() + "\n"


def _coordination_section(arms_dir, arm, sfx):
    if not roles_mod.is_role_arm(arm):
        return _load(arms_dir, "%s%s.md" % (arm, sfx)).strip()
    if sfx:
        raise ValueError("the role arm %s has no shared-tree (T-shared) variant" % arm)
    # K3 and B1-roles: the base arm's text, then the worker section of the role file; the integrator and supervisor
    # prompts go to the role members only (roles.role_prompt)
    return _load(arms_dir, roles_mod.base_arm(arm) + ".md").strip() + "\n\n" + roles_mod.worker_text(arms_dir, arm)


def build_prompt(arms_dir, arm, name, team, task, workdir, test_command, klatch_bin, room, max_turns, max_minutes,
                 restart_command="", topology="T-clone"):
    """The common prompt with the arm's coordination section, every placeholder filled, draft comments removed. For topology
    T-shared (#1427) the files common-shared.md and <arm>-shared.md are used: agents share one working directory."""
    sfx = "-shared" if topology == "T-shared" else ""
    common = re.sub(r"^# Common part[^\n]*\n", "", _load(arms_dir, "common%s.md" % sfx)).lstrip()
    text = common.replace("{{COORDINATION_SECTION}}", _coordination_section(arms_dir, arm, sfx))
    values = {"NAME": name, "TEAM_SIZE": str(len(team)), "TEAM_NAMES": ", ".join(n for n in team if n != name),
              "WORKDIR": workdir, "TASK": task, "TEST_COMMAND": test_command, "KLATCH": klatch_bin or "",
              "ROOM": room, "MAX_TURNS": str(max_turns), "MAX_MINUTES": str(max_minutes),
              "RESTART_COMMAND": restart_command}
    for k, v in values.items():
        text = text.replace("{{%s}}" % k, v)
    left = re.findall(r"\{\{[A-Z_]+\}\}", text)
    if left:
        raise ValueError("unfilled placeholders in the %s prompt: %s" % (arm, ", ".join(sorted(set(left)))))
    return text
