Subagents and hooks, measured in a live headless run (2026-11-21, installed Claude Code, ticket #581)
Results of a live run for #581 and #1152: headless `claude -p --model haiku --setting-sources local`, command hooks for SubagentStart, SubagentStop, PreToolUse, PostToolUse, Stop, UserPromptSubmit and SessionStart, three sessions, about 0.009 USD in all, no klatch in the run. This is the measured part of the subagent questions; it is cited as "the research note for #581" until assembled (it extends the subagent discussion that section 54 of the plan refers to).

### SubagentStart and SubagentStop hook inputs

- SubagentStart: `agent_id`, `agent_type`, `cwd`, `hook_event_name`, `prompt_id`, `session_id`, `transcript_path`. No `permission_mode`.
- SubagentStop: `agent_id`, `agent_type`, `agent_transcript_path` (basename `agent-<agent_id>.jsonl`), `transcript_path` (the parent's), `session_id` (the parent's), `permission_mode`, `last_assistant_message`, `stop_hook_active`, `background_tasks`, `session_crons`, `effort`, `cwd`, `prompt_id`, `hook_event_name`.
- A subagent's own tool calls: PreToolUse and PostToolUse carry `agent_id` and `agent_type` besides the usual fields; `session_id` is the parent's. The parent's own tool calls, including its Agent call, have neither key.
- Order seen: PreToolUse(Agent), SubagentStart, the subagent's PreToolUse and PostToolUse for Bash, SubagentStop, PostToolUse(Agent), the parent's Bash, Stop.

### What a subagent inherits

- Hooks: yes. The session's hooks fire for the subagent's tool calls (that is how `agent_id` reaches them); SubagentStart and SubagentStop fire from the parent's settings. No SessionStart or UserPromptSubmit fires for the subagent (each fired once, for the parent).
- Permission mode: yes. The subagent's hook inputs showed the parent's `acceptEdits`, and the Bash allow rule applied to the subagent as well.
- Environment: identical to the parent's. The variable names match, `CLAUDE_CODE_SESSION_ID` is the parent's, no agent id variable exists (`CLAUDE_CODE_AGENT_ID` is unset), and the process chain is the same `claude` process with no process of its own for the subagent. So a command a subagent runs cannot learn that it is a subagent from its environment or its parents; only a hook can (see the design item for #1152).
- Working directory: the same project directory.

### SubagentStart additionalContext reaches the subagent

A SubagentStart hook that printed `{"hookSpecificOutput":{"hookEventName":"SubagentStart","additionalContext":"..."}}` reached the subagent: it replied with a marker word it could only have learned from the hook. A per-subagent convention text or identity line can therefore be injected at start, and the hook knows `agent_id` and `agent_type` to build it.

### Foreground versus background lifetime

- Foreground (the default): the subagent starts and stops inside the parent's turn; SubagentStop comes before the parent's Agent PostToolUse and Stop.
- Background (`run_in_background` true): the parent's Stop fired before SubagentStop (15:49:01 and 15:49:02 in the run). The subagent outlives the parent's turn, and a completion notification then starts a second parent turn (a second UserPromptSubmit and Stop). In `-p` mode the process stayed alive until that second turn finished.

### Not run

- Isolation modes (worktree and remote subagents).
- Agent teams.
- MCP server inheritance by a subagent.
