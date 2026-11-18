# klatch beside the orchestrators: where it sits, where it overlaps

This page is for someone who already runs an orchestrator or agent-team tool and wants to know what klatch adds under it.
It says only what the repository's own research and public descriptions support, and it says which. **None of the tools
below has been installed or run against klatch**, so there is no tested recipe for any of them and every status line says
`not run`. A claim about a third-party tool is a description from a source named in its section, never a measurement; a
version, a flag or an option of a tool is not stated because none was checked. Running them is ticket #414 and needs a
person with the desktop tools; ticket #126 stays open until it is done.

What does run is the launcher pattern every one of them could use: `FACTORY.md` (the order of a launcher's work),
`clients/recipes/launcher-demo.sh` (the whole flow with no model, in CI), `clients/recipes/claude-code-worker.py` (a headless
Claude Code worker as a verified member, run once). klatch is not an orchestrator: it does not plan, dispatch, schedule or decide what merges (`DESIGN.md`
section 20 rule 14). It is the shared room, the claims and the refusals underneath whatever does.

## The overlap table

| Tool | What it is, as described | Where klatch overlaps | What klatch adds | What klatch does not do |
|---|---|---|---|---|
| Claude Code agent teams | an experimental feature: a lead session spawns and messages teammate sessions | messages between sessions, a shared task list | a room that outlives a session and can be joined by sessions that are not teammates; claims on paths; history for a late joiner; refusals and an audit | spawn or schedule a teammate |
| Orchestrator.inc | an orchestrator agent that starts workers in git worktrees and routes feedback to the worker that owns it | starting workers, routing feedback | claims on paths and work items across workers, handoffs with a kind and a payload | decide what work exists or route CI results |
| Claude Squad | a manager of several agent sessions, each in its own workspace | managing parallel sessions | a shared room between those sessions and claims so two do not edit one file | manage a terminal, a workspace or a worktree |
| Conductor | a desktop tool that runs several coding agents in parallel workspaces | managing parallel sessions | the same as above | manage a workspace |
| Agent Swarm | a multi-agent swarm tool | handing a task from one agent to another | a durable log, labels and clearance, a refusal record | plan or run a task graph |
| General agent frameworks | libraries for building agents (LangGraph, CrewAI, the OpenAI Agents SDK, the Microsoft Agent Framework) | agents talking to each other | a protocol and client libraries any of them can use as the shared room | build or host an agent |

The gap that matters most, from the same research: none of the orchestrators found enforces anything *between* two agents (a
claim on a path, a clearance on a room, a refusal for a secret in a message). That is klatch's part.

## Claude Code agent teams

Status: not run

What the documentation says (read for `RESEARCH-NOTES.md` section 2): one team per session, the team cannot be shared or
joined by an independently started session, there is no broadcast, no room and no history for a late joiner, and two
teammates editing one file overwrite each other (the page tells the user to give each teammate its own files). Each teammate
is a separate Claude Code instance that loads the project's instructions, MCP servers and skills.

How klatch would sit beside it, **unverified**: klatch's hooks are installed by `klatch init` into the project's Claude Code
settings, so an instance started in that project would be expected to join the lobby; whether a teammate's instance runs
those hooks was not tested. If it does, the lobby is the room the team lacks, and a claim on a path is the lock the page says
is missing for file edits. Fallback that needs no hooks: tell each teammate in its spawn prompt to run the commands in
`INTEGRATION.md` or `CONVENTIONS.md` itself.

Source: `RESEARCH-NOTES.md` section 2 (documentation read on 2026-11-12), `SURVEY.md` section 2.2.

## Orchestrator.inc

Status: not run

As described in public material gathered on 2026-11-12: an orchestrator agent spawns workers in worktrees and routes CI and
review feedback back to the owning worker. No source was read beyond the descriptions.

How klatch would sit beside it, **unverified**: the orchestrator is the launcher, so `FACTORY.md` applies as written: it
issues each worker a credential (`klatch worker issue`), starts the worker with only the bundle's path, and releases a
worker's anchored claims when the item is done (`klatch anchor`). Worktrees of one repository share a claim scope
(`DESIGN.md` section 27), so two workers in two worktrees of the same repository do conflict on the same path, which is the
point.

Source: `RESEARCH-NOTES.md` section 16 (third-party and vendor material; verify before relying on it).

## Claude Squad

Status: not run

As described in the same research: a manager of many agent sessions, each in its own workspace; the survey lists its licence
as AGPL-3.0 (`SURVEY.md` section 3). klatch would not link to it or ship any of its code; the licence matters only for someone
who modifies and redistributes it.

How klatch would sit beside it, **unverified**: each session it starts is a coding session in a repository, so `klatch init`
in that repository and the lobby convention apply as for any session; the sessions then share the room and the claims.

Source: `RESEARCH-NOTES.md` section 16, `SURVEY.md` section 3.

## Conductor

Status: not run

As described in the same research, a tool that runs several coding agents in parallel workspaces. Nothing more about it was
read for this repository; the survey notes that another tool's supervisor uses the same word, so a reader should check
which "Conductor" is meant before relying on a row.

How klatch would sit beside it, **unverified**: as for Claude Squad.

Source: `RESEARCH-NOTES.md` section 16, `SURVEY.md` section 3.

## Agent Swarm

Status: not run

As described in the same research: multi-agent task graphs and swarms. The description is too thin to say how it would start
or talk to an agent, so no setup is offered.

How klatch would sit beside it, **unverified**: through the client libraries (`clients/README.md`) if the swarm's agents can run
a program or open a pipe, or through the launcher pattern if it starts processes.

Source: `RESEARCH-NOTES.md` section 16.

## General agent frameworks

Status: not run

LangGraph, CrewAI, the OpenAI Agents SDK and the Microsoft Agent Framework build agents; they do not wrap a coding command
line. The survey records them as frameworks (`SURVEY.md`), and none was used here.

How klatch would sit beside them: an agent built with any of them can speak the wire protocol (`PROTOCOL.md`) through
the Python, TypeScript or Rust client, or run the `klatch` command, so a framework agent and a coding session share one room.
That much follows from the protocol being plain newline-delimited JSON over a pipe, which `clients/python` and
`clients/examples` exercise; it was not tried from any of these frameworks.

Source: `SURVEY.md`, `PROTOCOL.md`, `INTEGRATION.md`.

## How to verify a tool and what to report

For ticket #414, per tool: install it from its own documentation (not from this page), start two of its agents in one
repository with `klatch init` done, and check three things. Did both join the lobby? Did a claim by one refuse a claim by the
other on the same path? Did a message from one reach the other? Record the tool's version as you found it, the exact steps and
the result in `RESEARCH-NOTES.md`, change that tool's status line here to `Status: run`, and only then write its recipe. A
recipe that was not run does not go on this page.
