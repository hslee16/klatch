"""Agent drivers: how one agent of a team is started (SPEC 2.1).

A driver turns (agent name, arm, prompt, working directory) into an argv and an environment. The harness starts the
process, tails its stdout as stream-json and never speaks to it again. Two drivers:

  fake    a scripted deterministic agent (fake_agent.py) that edits files and runs git and klatch commands from a JSON
          script, with configurable failures. No model, no network, no spend: the whole harness runs in CI-light tests.
  claude  headless `claude -p` sessions with the model id from the config. The harness tests only build the command
          and parse recorded output; a real run needs the human's budget approval (run.py enforces a flag for it).

Environment rule for both: an allowlist, never a copy of the harness environment. The model credential is passed only
to the claude driver, and only if the harness environment has it; it is never printed or written anywhere.
"""
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
BASE_ENV_KEYS = ("PATH", "LANG", "LC_ALL", "TERM")
GIT_TOOLS = ["Bash(git:*)", "Bash(python3:*)"]
FILE_TOOLS = ["Read", "Edit", "Write", "Glob", "Grep"]
READ_TOOLS = ["Read", "Glob", "Grep"]
MESSAGING_ARMS = ("B1", "B1-roles")


def parse_claude_version(text):
    m = re.search(r"\d+\.\d+\.\d+(?:[-.][0-9A-Za-z.]+)?", text or "")
    return m.group(0) if m else None


class Driver:
    name = "driver"
    passthrough = ()
    extra_env = {}

    def agent_env(self, base, scratch, agent=None):
        """The environment of an agent process: an allowlist taken from the harness environment, plus what the
        driver passes. The run's HOME, KLATCH_HOME and KLATCH_KEY_DIR are added by the harness, not here."""
        env = {k: base[k] for k in BASE_ENV_KEYS if k in base}
        for k in self.passthrough:
            if k in base:
                env[k] = base[k]
        env.update(self.extra_env_for(scratch))
        return env

    def extra_env_for(self, scratch):
        return {}

    def command(self, name, arm, prompt, cwd, out_dir, session=1, max_turns=None, role=None, model=None, max_usd=None):
        """session is 1, or 2 for the restart of an agent whose context was cleared (S9 resume); max_turns overrides the
        driver's turn cap for this one session (the first session of a resume run is cut by it). role is "integrator" or
        "supervisor" for a role member of K3 or B1-roles (#1421); model and max_usd override the driver's model and its
        per-agent budget for this one session (a role has its own)."""
        raise NotImplementedError

    def version(self):
        return None


class FakeDriver(Driver):
    name = "fake"

    def __init__(self, script_path, klatch_bin=None, room="lobby"):
        self.script_path = script_path
        self.klatch_bin = klatch_bin
        self.room = room

    def command(self, name, arm, prompt, cwd, out_dir, session=1, max_turns=None, role=None, model=None, max_usd=None):
        argv = [sys.executable, os.path.join(HERE, "fake_agent.py"), "--script", self.script_path, "--name", name,
                "--arm", arm, "--out", out_dir, "--room", self.room, "--session", str(session)]
        if role:
            argv += ["--role", role]
        if self.klatch_bin:
            argv += ["--klatch-bin", self.klatch_bin]
        return argv, {}

    def version(self):
        return "fake"


class ClaudeDriver(Driver):
    name = "claude"
    passthrough = ("ANTHROPIC_API_KEY",)

    def __init__(self, claude_bin, model, max_turns, per_agent_usd, klatch_bin=None):
        self.claude_bin = claude_bin
        self.model = model
        self.max_turns = max_turns
        self.per_agent_usd = per_agent_usd
        self.klatch_bin = klatch_bin

    def extra_env_for(self, scratch):
        # the agent's tool state (its config directory) is per run, never the user's ~/.claude
        return {"CLAUDE_CONFIG_DIR": os.path.join(scratch, "claude-config")}

    def allowed_tools(self, arm, role=None):
        # a role member never edits the code (its prompt says so, and here it cannot): reading and the shell only
        tools = (READ_TOOLS if role else FILE_TOOLS) + GIT_TOOLS
        if arm.startswith("K") and self.klatch_bin:
            tools.append("Bash(%s:*)" % self.klatch_bin)
        if arm.startswith("B"):
            # waiting is a Bash call in a B arm (the K arms wait with `klatch wait`); no other shell is opened up
            tools.append("Bash(sleep:*)")
        if arm in MESSAGING_ARMS:
            tools += ["SendMessage", "ListAgents"]
        return tools

    def settings_for(self, arm):
        """The --settings JSON that decides native cross-session messaging (docs: cross-session-messaging.md). Only B1
        takes part: it accepts inbound messages unattended (a -p session cannot answer an approval dialog). Every other
        arm refuses inbound messages and has the two messaging tools removed, so nothing leaks between arms."""
        if arm in MESSAGING_ARMS:
            return {"crossSessionInbound": "accept"}
        return {"permissions": {"deny": ["SendMessage", "ListAgents"]}, "crossSessionInbound": "refuse"}

    def command(self, name, arm, prompt, cwd, out_dir, session=1, max_turns=None, role=None, model=None, max_usd=None):
        argv = [self.claude_bin, "-p", prompt, "--model", model or self.model, "--setting-sources", "local",
                "--strict-mcp-config", "--disable-slash-commands", "--no-session-persistence",
                "--permission-mode", "acceptEdits", "--allowedTools"] + self.allowed_tools(arm, role) + [
                "--max-budget-usd", "%.2f" % (self.per_agent_usd if max_usd is None else max_usd), "--max-turns", str(max_turns or self.max_turns),
                "--output-format", "stream-json", "--verbose", "--include-hook-events",
                "--name", name, "--settings", json.dumps(self.settings_for(arm), sort_keys=True)]
        return argv, {}

    def version(self):
        import subprocess
        try:
            p = subprocess.run([self.claude_bin, "--version"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                               text=True, timeout=30)
        except (OSError, subprocess.SubprocessError):
            return None
        return parse_claude_version(p.stdout)
