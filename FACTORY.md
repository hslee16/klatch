# klatch under a software factory

klatch is not an orchestrator, a scheduler, a workflow engine or a merge gate (DESIGN section 20 rule 14). It is the room
the workers of a factory share: who is here, who holds which path, what was handed to whom, and a durable record. A factory
launches the workers and decides what work exists; klatch carries the coordination and enforces the rules it is given.

## What a launcher does, in order

| Step | Command | What it gives |
|---|---|---|
| Open a run | `klatch run create <id>` | the rooms `run.<id>.*`; rooms of a run that is not open are refused |
| Issue workers | `klatch worker issue --count N --prefix P --clearance <label> --parent <member> --out-dir D` | one 0600 key bundle each: a unique member, a token the gateway can expire and revoke, the keys its clearance reaches |
| Start a worker | environment: `KLATCH_HOME`, `KLATCH_IDENTITY_FILE=<path of its bundle>` (no name, no key directory) | a verified member from its first action; the path, never the contents, is what the launcher hands over |
| Workers work | `post` (with `--kind` and `--payload` for a handoff), `claim <path> --anchor <work item>`, `read`, `wait`, or a client library | rooms, claims, handoffs, all under the label rules |
| An item finishes | `klatch anchor done <item>`, or `klatch anchor reap --dir D` (the factory writes a file per finished item) | the claims anchored to it are released and the paths are free at once |
| A worker ends | its SessionEnd hook revokes its own credential; `klatch worker revoke <member>` does it from outside | the token stops working at once |
| End the run | `klatch run end <id> --archive <file>` | an encrypted archive, then the run's rooms are removed |
| Read the archive | `klatch admin run-archive <file> --run <id>` (a person at a terminal; audited) | the transcript as JSON lines |

`clients/recipes/launcher-demo.sh` runs that whole flow with no model and checks every step; CI runs it. The client libraries
(`clients/`) speak the wire protocol from Python, TypeScript and Rust for a factory that is not written in Rust.

## Recipes that were run

- **A script or factory as launcher, workers as processes.** `clients/recipes/launcher-demo.sh`, above. Run in CI. Fifty
  workers at once: `proto/CONCURRENCY.md` (no message lost or doubled, one winner of a contested path, every message verified).
- **Claude Code as the worker.** `clients/recipes/claude-code-worker.py` starts a headless `claude -p` with only the bundle's
  path in its environment; it posts through its Bash tool and an observer sees `w-live` verified. Run on 2026-11-14 with
  Claude Code 2.1.325 (RESEARCH-NOTES section 40). Interactive Claude Code sessions use hooks (`klatch init`), the wake
  watcher and invites (DESIGN sections 25 and 31), which were run in the terminal (RESEARCH-NOTES section 39).

## What a bundle does and does not give

It gives an identity that is issued, expires, can be revoked and is audited, so "verified" on a message means a live credential
sent it, and it gives each worker only the sealed-label keys its clearance reaches. It does not isolate a hostile worker: with no
daemon the bundle holds the store key, so a hostile process of the same user that has the file can open the database. That
needs a separate OS user or a daemon (DESIGN section 32 item 13). Do not put a bundle in a repository, and hand a worker its
path, not its contents.

## Overlap and gaps with the orchestrators (desk research, not run)

These rows come from the public descriptions gathered in RESEARCH-NOTES section 16 on 2026-11-12. **None of these tools was
installed or run against klatch**, so none of this is a tested recipe; running them is ticket #414.

| Tool | What it is, as described | Where klatch overlaps | What klatch adds | What it does not do |
|---|---|---|---|---|
| Claude Code agent teams (experimental) | a lead session that spawns and messages teammate sessions | messaging between sessions, a shared task list | rooms and claims that outlive a session and span repositories; refusals; an audit | spawn or schedule anything |
| Orchestrator.inc | an orchestrator agent spawns workers in worktrees and routes CI and review feedback to the owning agent | spawning, routing feedback | claims on paths and work items across workers, handoffs with a kind and payload | decide what work exists or route CI results |
| Claude Squad, Conductor, dmux, Agentastic | managers of many parallel agent sessions, each in an isolated workspace | session management and isolation | a shared room between those sessions, claims so two do not edit one file, verified identity for workers it issues credentials to | manage a terminal, a workspace or a git worktree |
| Agent Swarm, Open Multi-Agent | multi-agent task graphs and swarms | task hand-off | a durable log, labels and clearance, a refusal record | plan or run a task graph |
| LangGraph, CrewAI, OpenAI Agents SDK, Microsoft Agent Framework | frameworks for building agents | agents talking to each other | a protocol and client libraries any of them can use as the shared room | build or host an agent |

The gap that matters most: none of the orchestrators found in that research enforces anything between two agents (a claim on a
path, a clearance on a room, a refusal for a secret in a message). That is klatch's part, and the reason to put it underneath
them rather than beside them.
