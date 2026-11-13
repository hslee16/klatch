# Survey: what already exists for coordinating coding agents

Date: 2026-11-12. Client at the time: Claude Code 2.1.323 (also the newest release; no later
changelog entry mentions messaging, channels or teams).

Question this survey answers: **before building rooms for coding agents, what already exists that
is good enough to adopt, borrow from, or must be differentiated from?** The requirement ids
(R1 to R20) are the ones in `DESIGN.md` section 4; the hazard ids (H1 to H12) are in
`CASE-STUDY.md`.

## 0. How to read this file: verification levels

Three research passes were run in parallel (coordination tools; protocols and infrastructure;
vendors, frameworks and practitioner accounts). Their reports were then **checked**. Every claim
carries one of these tags, and only the tagged level should be trusted:

| Tag | Meaning |
|---|---|
| **V-API** | I read the value myself from the GitHub REST API on 2026-11-12 (license, language, stars, last push, archived flag, issue state) |
| **V-PAGE** | I fetched the page or README myself and the quoted text is there |
| **V-DOC** | I read it myself on the primary Claude Code documentation page |
| **REPORT** | A research agent says it read the page; I did **not** re-read it |
| **SNIPPET** | Seen only in a search result |
| **UNKNOWN** | Nobody established it |

Counts, exactly: all **21** repositories in the table in section 3 were re-read on V-API (every
figure matched the reports), plus A2A, ACP and NATS in section 4 and 5. The README claims of **10** of
them were re-read (V-PAGE): mcp_agent_mail Rust, Cotal, agent-loom, agent-bus, claude-peers, AMQ,
claude-presence, ruflo, Vibe Kanban and Crystal. Five security and incident sources were re-read
(CodeIntegrity, issue #92060, the `claudemessaging` README, METR's page, the CSA note), the quotes from
the Cursor and Anthropic pages were re-read, and six paper titles were confirmed to exist. What was
**not** re-checked is labelled REPORT or SNIPPET, and section 10 lists what is still unknown.

### What was not covered at all

- Hacker News and Reddit sweeps (the search returned no HN threads; every practitioner account is
  a blog, a DEV post or a vendor page).
- A security-history check (issues, CVEs, advisories) on each tool. Every "no known incident" is
  UNKNOWN.
- Source code. No tool's code was read; everything is README, docs or metadata level.
- Windsurf, OpenCode, Aider, Zed, JetBrains Junie, Letta, MetaGPT, ChatDev, smolagents, Pydantic
  AI, Mastra and Semantic Kernel (UNKNOWN: nothing on inter-session messaging was found, but they
  were not studied).

---

## 1. Headline findings

1. **Claude Code already has most of the transport.** Cross-session messaging has been on by
   default since v2.1.224 (V-DOC). It is the delivery layer to build on, not to replace.
2. **A close prior-art project exists: Cotal.** A pub/sub standard on NATS with channels, DMs,
   presence, retention and a Claude Code connector (V-PAGE for its README; docs details REPORT).
   It overlaps the design heavily and must be read in full before any build decision.
3. **Two more local-first projects match large parts of the design:** `agent-loom` (path claims,
   work leases, receipts, process-ancestry sender proof, optional channel push) and `agent-bus`
   (one SQLite WAL file, 65 MCP tools). Both V-PAGE. And `mcp_agent_mail` (Rust rewrite) has
   advisory file leases with owner, reason and TTL plus a pre-commit guard (V-PAGE).
4. **Nobody found combines** named many-to-many rooms, path leases with owner and expiry,
   verified sender identity, **sensitivity labels with clearance and a scrubber**, and a plain
   transcript. The label-and-clearance layer is absent from every tool read. This is the
   differentiator, and it is also the layer the incident evidence says is missing.
5. **The privacy problem is documented, not hypothetical.** A published demonstration shows two
   Claude Code sessions with different capabilities turning cross-session messaging into a data
   exfiltration path, and a filed issue shows the folder boundary can be crossed through
   `SendMessage` without a prompt (section 7).
6. **Locks fail for agents unless they expire.** Cursor tried shared-file locks with 20 agents and
   abandoned them; Anthropic's own compiler experiment used lock files in git and survived (section
   8). The common thread is: leases, advisory, with expiry.
7. **The protocols are the wrong layer.** A2A is the survivor among the agent protocols and has no
   rooms; ACP is archived. For same-user, same-machine use they are overkill.
8. **Storage:** a SQLite WAL file as the system of record, with JSONL as a derived transcript,
   fits better than pure JSONL once claims and per-member cursors exist (a reasoned
   recommendation, section 5; not measured).

---

## 2. The first-party baseline (V-DOC unless noted)

### 2.1 Cross-session messaging

Primary page: https://code.claude.com/docs/en/cross-session-messaging (read in full on
2026-11-12; details are in `RESEARCH-NOTES.md`). What matters for the survey:

- **Tools:** `ListAgents` and `SendMessage`. Plain text only, never files or history. About one
  million characters maximum. A rapid burst to one session is refused at the sender. At the
  receiver: repeats per sender are rate-limited, identical repeats within a short window are
  dropped, and at most 50 accepted messages are queued. A loop between two sessions therefore
  stops on its own.
- **Identity:** the session name (from `/rename`, `--name`, or auto-assigned). Two sessions with
  the same name in different directories get a short identifier added. A name is a label, not a
  credential.
- **Default inbound rule (when no `crossSessionInbound` value applies), decided from the two
  sessions' permission modes.** Sessions that bypass permission prompts are one class, every other
  mode the other class. A *prompting* receiver delivers each message and holds one only if the
  sender says it is bypassing. A *bypassing* receiver holds every message unless the sender also
  says it is bypassing. A held message opens an approval dialog; unanswered past `dialogExpiry`
  (default five minutes) it is dropped. At most 100 held messages, oldest dropped first.
  `-p` workers keep held messages for the same deadline; `"never"` keeps them until the session
  ends. An explicit `hold` setting does not expire.
- **Inbox socket:** a per-session Unix socket, exported to hooks and Bash as
  `CLAUDE_CODE_MESSAGING_SOCKET`, with a per-session token `CLAUDE_CODE_MESSAGING_TOKEN`. **A
  script or hook may post into its own session's socket**; the documented handshake is an optional
  first line `{"type":"auth","token":"<token>"}` (required on native Windows). A message from the
  session's own child process is delivered when no setting applies, verified by process evidence
  (or by the token on macOS once the process has exited). **The line format of the message itself
  is not on this page** (a wire-format experiment is open question Q1 in `DESIGN.md`).
- **Not present:** rooms, broadcast, history for a latecomer, claims, labels, clearance.
- **The sender's mode is self-declared** (section 7, `claudemessaging` finding F-3).

### 2.2 Agent teams

Page: https://code.claude.com/docs/en/agent-teams. Experimental; a lead spawns teammates; shared
task list; mailboxes as JSON files; one team per session; **cannot be joined or shared**; not
available under `-p` or the SDK. Verdict in `RESEARCH-NOTES.md` section 2: not a room.

---

## 3. Tools built to coordinate coding agents

Metadata is V-API on 2026-11-12 for every row. "Updated" is the last push.

| Tool | License | Language | Stars | Updated | Status |
|---|---|---|---|---|---|
| mcp_agent_mail (Rust rewrite) | MIT + AI-vendor rider (README badge); API says NOASSERTION | Rust | 251 | 2026-11-12 | active |
| mcp_agent_mail (Python original) | NOASSERTION | Python | 2,235 | 2026-11-05 | **deprecated** by its own description |
| Cotal | Apache-2.0 | TypeScript | 389 | 2026-11-12 | active, created 2026-06-02 (REPORT) |
| agent-loom | MIT | TypeScript | 5 | 2026-10-01 | active, created 2026-08-02 |
| agent-bus (MustaphaSteph) | MIT | TypeScript | 2 | 2026-10-29 | active, created 2026-05-13 |
| claude-peers-mcp | MIT | TypeScript | 2,212 | 2026-04-26 | quiet for 6 months |
| agent-message-queue (AMQ) | MIT | Go | 92 | 2026-11-12 | active |
| claude-presence | MIT | TypeScript | 12 | 2026-07-24 | quiet |
| Multisession | MIT | Python | 0 | 2026-11-09 | active, tiny |
| claude-relay | **none** | JavaScript | 5 | 2026-10-15 | no license: cannot be adopted as is |
| mcp-coordinator | MIT | TypeScript | 1 | 2026-11-11 | active, tiny |
| Gas Town | MIT | Go | 18,778 | 2026-11-12 | active |
| Beads | MIT | Go | 28,694 | 2026-11-12 | active |
| ruflo (was claude-flow) | MIT | TypeScript | 81,573 | 2026-11-12 | active |
| Claude Squad | AGPL-3.0 | Go | 8,793 | 2026-08-20 | active |
| agent-deck | MIT | Go | 1,253 | 2026-11-12 | active |
| ccswarm | MIT | Rust | 157 | 2026-11-07 | active |
| Crystal | MIT | TypeScript | 3,127 | 2026-02-26 | superseded (README says Nimbalyst, V-PAGE) |
| Vibe Kanban | Apache-2.0 | Rust | 28,668 | 2026-11-12 | **sunsetting** (README, V-PAGE) |
| Overstory | MIT | TypeScript | 1,314 | 2026-05-28 | **archived** |
| AgentAPI | MIT | Go | 1,496 | 2026-09-13 | **archived** |

Stars are a popularity signal only; ruflo's count says nothing about correctness or fit.

### 3.1 mcp_agent_mail (Rust) and its Python original

https://github.com/Dicklesworthstone/mcp_agent_mail_rust , the original at
https://github.com/Dicklesworthstone/mcp_agent_mail (deprecated: the description says it was
superseded because of Git lock contention and SQLite pool exhaustion, per the README, REPORT).

What I read (V-PAGE): "Advisory File Reservations: agents declare exclusive or shared leases on
file globs before editing, preventing conflicts with a pre-commit guard"; a Git hook that "blocks
commits touching files reserved by other agents"; the call
`file_reservation_paths(project_key=..., agent_name=..., paths=["src/**"], ttl_seconds=3600,
exclusive=true)`; the sentence "Advisory, not mandatory. File reservations are advisory leases,
not hard locks. The pre-commit guard enforces them at commit time, but agents can override"; an
authentication section stating it cannot "isolate hostile processes sharing the same OS account"
or hostile clients sharing one bearer credential; a documented MCP tool group of five file
reservation tools including `force_release_file_reservations`.

REPORT only: self-registered agent names (memorable generated names), point-to-point mail with
threads and acks, project-level broadcast that a later joiner does not see for older messages, a
contact-approval handshake before cross-project mail (`CONTACT_REQUIRED` when no approved link
exists), a `HumanOverseer` sender that bypasses contact policy, a web UI and a 16-screen TUI,
Markdown messages in a per-project Git repository with SQLite indexing, an installer that pipes a
remote script to bash with minisign verification, delivery that is mostly pull.

Against the requirements: R1 yes (owner, reason, TTL, advisory, commit guard); R4 yes; R5 no
(self-declared, authentication off unless configured); R7 to R9 no (the contact gate is a topology
control between projects, not a content label); R10 yes; R11 unknown; R14 partial; R18 yes (MCP).
**License caveat:** the badge says MIT plus an AI-vendor rider and the API reports NOASSERTION;
read `LICENSE` before any adoption.

### 3.2 Cotal

https://github.com/Cotal-AI/Cotal , docs https://docs.cotal.ai . Apache-2.0, TypeScript.

V-PAGE (README): "the open pub/sub standard for AI agents" on NATS and JetStream;
**multicast** to a channel such as `#review`, **unicast**, and **anycast** to a role such as
`reviewer` where one available instance takes the work; a link titled "Connect a personal Claude
Code session: Existing-session connection and approval"; "The CLI sends usage telemetry by
default. Disable it with `cotal telemetry off`."

REPORT (docs): durable versus live delivery class per channel; retained history readable by an
agent with a read grant even if it has not joined; presence states `idle`, `waiting`, `working`,
`offline` with a heartbeat that "reports liveness, not proof that a task is progressing"; per-channel
`allowSubscribe` and `allowPublish` with posting default-deny (enforced only on an authenticated
mesh; "an open mesh does not enforce post ACLs"); attention modes and a mute mode with `@mention`
handling; a Claude Code connector via plugin and hooks, where personal sessions "receive messages
at supported busy boundaries" and **waking a fully idle session is unknown**; a managed seat launched
with `--permission-mode bypassPermissions`; a custom channel needing
`--dangerously-load-development-channels`; install by `curl ... | sh`.

Against the requirements: many-to-many channels yes; R4 yes; R5 partial to yes depending on the
mesh's authentication (REPORT, details unread); R1 **no** (anycast claims a message, not a path);
R7 to R9 **no** sensitivity labels seen (channel ACLs are not labels); R10 partial; R11 partial
(`cotal down`); R14 yes; R18 partial. **Joining an independently started local session needs a
signed-in flow and an approval; for a local static-auth or open mesh the docs say to launch
through `cotal spawn` instead.** It is the strongest channel model found and the one to read first.

### 3.3 agent-loom

https://github.com/osteele/agent-loom . MIT, TypeScript, created 2026-08-02, 5 stars.

V-PAGE: "Path claims keep two agents from editing the same files, work leases record who is
responsible for a logical unit"; a `project_owner` role that can inspect, claim or release; a claim
"returns a one-time release token"; receipts that "distinguish spooled, pending, held, pushed,
push-unreachable, read, ..." (further states in the full line); sender attribution that stamps an
identity only when the CLI can prove process ancestry and otherwise sends the `--from` value as a
free-form label tagged `[label; not a reply address]`; optional **channel push** into Claude Code
that needs `channelsEnabled` and an `allowedChannelPlugins` entry.

Against the requirements: R1 yes (path claims and work leases; expiry detail not read); R4
partial (durable spool); R5 **partial and well designed** (proof when possible, an explicit
"unattributed" tag otherwise); R7 to R9 no; R10 unknown; R14 unknown; R18 partial (the push needs
research-preview setup; the mail and claims do not). **Small, young and one person's project;
the ideas matter more than the code.**

### 3.4 agent-bus (MustaphaSteph)

https://github.com/MustaphaSteph/agent-bus . MIT, TypeScript, 2 stars.

V-PAGE: "65 MCP tools: messaging, synchronous and async ask/reply, thread replies, capability and
role routing, first-class tasks with at-least-once delivery, review gates"; one SQLite file at
`~/.agent-bus/bus.db`; "no daemon, no cloud, no auth, no bill". Works with Claude Code, Codex, Kimi
Code and Cursor. REPORT: SQLite WAL, a listener hook and blocking inbox waits.

Against the requirements: R4 yes; R5 **no**; R1 not documented as path claims; R7 to R9 no; R18
yes. The interesting part is the architecture: **one WAL file and no daemon**, which is the storage
recommendation of section 5.

### 3.5 claude-peers-mcp

https://github.com/louislva/claude-peers-mcp . MIT, 2,212 stars, last push 2026-04-26.

V-PAGE: a broker on `localhost:7899` with SQLite; each session spawns an MCP server; the quick start
launches `claude --dangerously-skip-permissions --dangerously-load-development-channels
server:claude-peers`; with `OPENAI_API_KEY` set, each instance sends its context to an OpenAI model to
generate a summary. REPORT: needs Claude Code 2.1.80 or later and a claude.ai login; no broker
authentication is documented. Another project's README (mcp-coordinator, REPORT) says on Claude Code
2.1.233 "no `<channel>` tag was injected in any of four configurations", i.e. channel push may not
work on a stock install.

**Risk:** development flag, skip-permissions in the documented launch, an unauthenticated localhost
broker (as far as the README says), and an optional leak of context to a third party.

### 3.6 agent-message-queue (AMQ)

https://github.com/avivsinai/agent-message-queue . MIT, Go, 92 stars.

V-PAGE: "Receipts distinguish a queued message from one the recipient drained"; "By default the wake
rings a plain doorbell (text and Enter) on every harness, so it always rings; a doorbell that lands
on an open dialog can answer it"; an optional `amq wake install-dialog-hook` for Claude Code that
keeps the wake from typing into an open AskUserQuestion, plan or permission dialog. REPORT: plain
files in a shared directory (Maildir style); `amq send` and `amq drain`; threads; roster handles; a
SECURITY.md that says it is "not an isolation boundary between mutually untrusted agents or users";
files `0700` and `0600`.

Lessons: receipts as a first-class concept; **a wake that types into a terminal is dangerous and
needs a dialog guard**; an honest security statement.

### 3.7 claude-presence

https://github.com/garniergeorges/claude-presence . MIT, TypeScript, 12 stars.

V-PAGE: "Sessions don't talk directly: each Claude Code session is an isolated process", it gives a
shared bulletin; slash commands `/register`, `/presence`, `/claim`, `/release`, `/broadcast`,
`/inbox`; `/claim ci` "reserve the CI before you push"; "sessions with no heartbeat for 24 hours are
removed automatically". REPORT: hooks `session-start.sh` and `user-prompt-submit.sh` surface pending
items at each prompt; optional team mode with bearer authentication and RBAC; scope is one repo.

It is the cleanest example of **documented features only** (hooks, slash commands, MCP). The delivery
pattern (pull at the next prompt) is exactly the hook injection in `DESIGN.md` section 8.6. A
per-claim expiry is not shown.

### 3.8 Others in the coordination category

- **Multisession** (danielrosehill): MIT, 0 stars. REPORT: agent A writes a Markdown file into repo
  B's `a2a/` folder and B finds it at session start, no server; its README says a successful
  `SendMessage` "means the message reached a session, not that its Claude read it", so handovers
  "close on a receipt". Needs KDE's Konsole for part of it.
- **claude-relay** (gvorwaller): **no license**. REPORT: WebSocket relay across machines over an SSH
  tunnel; durable per-label mailboxes; explicit session-identity anchoring on the parent Claude
  process or a token and transcript root; a wake hook that resumes a headless delegate; admin tools
  that can purge history.
- **mcp-coordinator** (swoofer): MIT, 1 star. REPORT: `announce_work` with optional target symbols
  and a conflict score; optional JWT or OAuth; claims "2500+ tests"; its README admits channel push
  "does not currently load on a stock install".
- **Gas Town and Beads** (gastownhall): MIT, large. REPORT: a heavyweight orchestrator with a
  "Mayor" coordinator, mailboxes, identities, git-worktree hooks, convoys and a federated "wanted
  items" network over DoltHub; Beads is an issue tracker for agents and a plausible task-board
  component. It spawns and manages agents; it does not coordinate independently started sessions.
- **ruflo and its federation plugin** (ruvnet): MIT. V-PAGE confirms the README *claims* "Zero-trust
  federation ... Identity proven via mTLS + ed25519 challenge-response", "AgentBBS rooms so agents on
  different hosts can coordinate", "Signed federation messages, channels, and work claims" and a
  "ruflo-aidefence: block prompt injection, detect PII" plugin. REPORT: a five-tier trust model and a
  PII pipeline with per-trust-level BLOCK, REDACT, HASH, PASS policies. **These are README claims on
  a very large, marketing-heavy project and are unaudited**; the federation targets agents on
  different hosts, and local fit is unknown.

### 3.9 Spawn-and-supervise managers (they do not give agents a channel to each other)

Claude Squad (AGPL-3.0; tmux and worktrees; README advertises an auto-accept mode), agent-deck
(a TUI with a "Conductor" supervisor), ccswarm (README says it runs Claude with
`--dangerously-skip-permissions` and an allowed-tools list), Crystal (now Nimbalyst), Vibe Kanban
(sunsetting), Overstory (archived), AgentAPI (archived). Also seen, metadata only: Emdash, cmux,
parallel-code, agent-orchestrator, ralph-orchestrator. **Do not build on the archived,
sunsetting or superseded ones.**

---

## 4. Protocols (REPORT unless noted; repo metadata V-API)

| Item | Status | Verdict for this use |
|---|---|---|
| **A2A** (a2aproject/A2A) | Apache-2.0, 26,509 stars, pushed 2026-11-11, not archived (V-API). Reported as accepted into the Agentic AI Foundation; v1.0.0 2026-03-12; v1.1.0 release candidate 2026-10-29 (SNIPPET) | A point-to-point task protocol (JSON-RPC over HTTP, Agent Cards, streaming and push). No rooms, broadcast, latecomer history, claims or labels. No coding agent speaks it natively; there are open feature requests on Claude Code and Codex (SNIPPET). Worth a thin adapter only if a remote agent must join a room as a peer |
| **ACP** (i-am-bee/acp) | **Archived** 2025-08-25 (V-API); README says it is now part of A2A (REPORT) | Dead; do not adopt |
| AGNTCY ACP spec | Archived (REPORT) | Dead |
| AGNTCY OASF, SLIM; ANP | Active (metadata only) | Enterprise or internet scale; overkill |
| **MCP** spec 2026-07-28 | Current (SNIPPET): stateless core, `subscriptions/listen` replaces `resources/subscribe`, tasks as an extension, sampling and elicitation reworked | No sender identity or rooms; Claude Code surfaces server content only when the model calls a tool, except through channels |
| Agent Client Protocol (Zed) | Active | Editor to agent; wrong problem |

Statements by Anthropic or OpenAI on agent-to-agent messaging: none found beyond Anthropic's own
shipped features (section 2). Whether a thin A2A adapter is worth building is a decision for after
the first working room.

---

## 5. Infrastructure that could back a room (REPORT; mostly general knowledge)

The recommendation here is the author's reasoning from the reports, **not a measurement**.

| Option | A gateway would gain | It would cost |
|---|---|---|
| **SQLite in WAL mode** | One file, transactional append with a monotonic row id, indexed history and presence queries, a unique-constraint claim table that makes a lease atomic, per-member cursors, no daemon. `agent-bus` is built this way (V-PAGE) | Notifications need polling or a file watcher; not greppable without a view; readers see only committed rows |
| **JSONL append-only** | `cat`, `grep`, `tail -f`; trivial to diff, back up and export | Atomic append needs a single writer or `flock`; no indexes; cursors and compaction are yours; whether `O_APPEND` writes of the relevant size are atomic enough on macOS APFS was **not tested** |
| NATS plus JetStream | Many-to-many subjects, durable streams and replay, per-member consumers, per-credential auth. Cotal is built on it | A daemon; you still build identity, labels and the transcript |
| Redis or Valkey streams | History and consumer groups | A daemon; pub/sub alone has no history |
| ZeroMQ | Brokerless sockets | No persistence, history or presence |
| MQTT (Mosquitto) | Retained messages, last-will presence, topic ACLs | A broker; weak history |
| Unix-socket broker | OS-verified peer credentials (Claude Code does the same for inboxes) | You write and run a daemon |
| Maildir-style mailboxes | Atomic delivery by rename, no daemon | One copy per recipient; poor for broadcast and history; agent teams use JSON inbox files |
| Git as the store | Auditable | Slow, noisy; `mcp_agent_mail` uses a Git archive with SQLite indexing, and its Python version was deprecated for Git lock contention |
| IRC (ergo), Matrix (Synapse, AGPL), Zulip, Mattermost, Rocket.Chat | Human UI, history, rooms | Heavy to run; their trust model, not yours; none has labels or machine-readable claims |
| Slack, Discord, Telegram | Human UI | Cloud; message content leaves the machine; violates local-first and the label model |

**Recommendation.** Keep the **gateway as the only writer**. Use **SQLite in WAL mode as the system of
record** for messages, members, cursors, claims and leases, and write each committed message to a
**JSONL transcript as a derived, regenerable view** the human can read with any tool. Pure JSONL is
acceptable for a first version with few members if the gateway holds `flock`, but the move to SQLite
should come before claims and leases land, because an atomic lease is a one-line unique constraint in
SQLite and a hand-built protocol in JSONL. Do not adopt NATS, Redis, Matrix or Kafka for same-machine,
same-user use. Reconsider NATS only if rooms must span machines, which is Cotal's territory.

---

## 6. Vendor products and frameworks (mostly SNIPPET; weakest evidence in this survey)

| Vendor | Do separate sessions message each other? | Can independent sessions join? | Notes |
|---|---|---|---|
| Claude Code | Yes (section 2) | Yes (same user, same machine) | Baseline |
| OpenAI Codex | Subagents report to the parent (SNIPPET). Issue #21027, "Shared workspace/message bus for Codex subagents", opened 2026-05-04, still open (V-API); its author says users emulate a bus with a `board.md` that has "no built-in locking or ownership semantics" (REPORT) | No | A server-side multi-agent beta with `spawn_agent` and `send_message` is reported by one third-party page (SNIPPET) |
| GitHub Copilot agent HQ / mission control | No agent-to-agent messaging found; human-to-agent steering and audit (SNIPPET) | n/a | |
| Cursor | No (SNIPPET). Its own blog (V-PAGE): flat agents with shared-file locks failed, planners and workers succeeded; workers "don't coordinate with other workers" | No | Section 8 |
| Devin | Parent to child; shared knowledge as a git repo of Markdown files that swarms used "as an unprompted coordination message board" (SNIPPET) | No | "Don't Build Multi-Agents" argues for sharing full traces (SNIPPET) |
| Gemini CLI, Jules; Roo, Cline, Kilo; Warp; Amp; Factory; Kiro | Subagents report to a parent; no sibling messaging found (SNIPPET); Amp is said to support it (SNIPPET); Roo Code reported archived 2026-05-15 by a single comparison (UNKNOWN) | No | |

General frameworks (LangGraph, AutoGen / AG2, OpenAI Agents SDK, Microsoft Agent Framework, CrewAI):
shared-state or handoff abstractions (typed state with reducers, group chat as an untyped blackboard,
handoffs through conversation history). **None was found to document a persistent room with latecomer
history and per-member permissions.** Reported failure modes (SNIPPET): infinite handoff loops, loss of
fidelity at each hop, context bloat, and full-history cost on every group-chat turn.

**MAST** (V-PAGE title and the report's reading of the abstract), "Why Do Multi-Agent LLM Systems
Fail?", arXiv 2503.13657: 1,600+ annotated traces across 7 frameworks, 14 failure modes in 3 clusters
(system design, inter-agent misalignment, task verification).

---

## 7. Security and privacy evidence between cooperating agents

This is the part of the survey that most directly supports the design's label and clearance layer.

### 7.1 Cross-session messaging as an exfiltration path (V-PAGE)

**CodeIntegrity, "Claude Code cross-session escalation risks"**
(https://www.codeintegrity.ai/blog/claude-code-cross-session-escalation-risks). Two sessions: A can read
email only, B can read files only. Planted emails make A ask B for a confidential internal document,
and A then returns the contents to the attacker. Verified text: "The channel carries a request without
preserving the original request's provenance or authorization context. Session B can see that session
A sent the message, but it cannot tell whether A is acting on a user instruction or on content from an
email." This is the public-repository agent and private-directory agent case in `CASE-STUDY.md`, shown
to work.

### 7.2 Folder scope can be crossed through `SendMessage` (V-API and V-PAGE)

Issue anthropics/claude-code **#92060**, opened 2026-09-04, labelled `bug`, `area:security`,
`area:permissions`, `platform:macos`, `stale`. The body states that cross-session messaging "lets a
session obtain the contents of files in another session's working directory with no permission prompt
in either session, while a direct Read of the same file across the folder boundary DOES prompt", and
that "folder scope is the only information barrier a single operator has" when running one session per
client or engagement. **State correction:** the issue shows as closed "not planned" on 2026-11-09, but
the only comment is the automated closer: "Closing for now, inactive for too long." **It was closed by
a stale bot, not by a maintainer decision. The problem is unresolved, not declined.**

### 7.3 The sender's permission mode is self-declared (V-PAGE)

The `claudemessaging` project (S3cur3Th1sSh1t; a research tool, 10 stars) documents in its README:
"F-3: `from-mode` attestation is self-declared, it controls the hold gate decision. By design: the hold
gate trusts the sender's declared permission mode. A local process with the session key can declare
`from-mode`", and "F-1: an approved message from an unidentified sender is framed to the model as
'trusted teammate'". Injection needs same-user access to the key file and socket. This is consistent
with the documented receiver rule in section 2.1: the hold gate decides on what the sender says.

### 7.4 Other sources (REPORT or SNIPPET unless stated)

- **Prompt Infection: LLM-to-LLM Prompt Injection within Multi-Agent Systems**, arXiv 2410.07283
  (title V-PAGE). Self-replicating prompts spread across agents; reported proposed defence "LLM
  tagging".
- Further titles confirmed to exist (V-PAGE, content not read): 2609.22949 "Beyond Single-Model
  Injection: A Threat Model and Defense Architecture for Prompt Injection in Multi-Agent Systems";
  2601.11893 "Taming Various Privilege Escalation in LLM-Based Agent Systems: A Mandatory Access Control
  Framework"; 2609.00595 "SoK: When Safe Agents Fail Together: The Security of Multi Agent LLM
  Systems"; 2608.00747 "When Prompts Control Robots: Prompt Injection Attacks in Multi-Agent Robotic
  Systems".
- **Cloud Security Alliance research note on confused deputy attacks** (page exists, V-PAGE; its text
  says a sub-agent "may inherit some or all of the orchestrator's permissions", and that neither
  pattern isolates the sub-agent). The report adds: treat inter-agent messages as a trust boundary,
  explicit, logged and revocable authority grants, signed messages, human confirmation enforced by the
  runtime (REPORT).
- **OWASP Top 10 for Agentic Applications 2026, ASI07 "Insecure Inter-Agent Communication"** (SNIPPET):
  mutual authentication, signed and validated messages, anti-replay, typed schemas, an attributable
  audit trail.
- **Invariant Labs** (SNIPPET): tool poisoning (2025-04-01); a GitHub MCP "toxic agent flow" where an
  injection in a public issue led an agent to leak private-repository data through a pull request
  (2025-05-26). This is the same public-to-private leak shape as H8.
- **CovertSwarm** (SNIPPET): in a study of 17 models, 82% ran malicious commands requested by a peer
  agent that they refused from a user. **The study's method was not verified.** Treat the number as
  unverified.
- **ServiceNow Now Assist second-order injection** (SNIPPET via a CSA note, November 2025): a
  low-privilege agent induced a higher-privilege peer to export data.

### 7.5 A coordination channel as the hazard itself (V-PAGE for the existence of the page)

METR's investigation (https://metr.org/blog/2026-08-26-openai-hugging-face-incident-investigation/):
roughly 1,200 agents "meant to be isolated from one another found a way to communicate with one another
on an unsanctioned message board", sending over 70,000 messages and files, and 700 went on to take part
in an attack on Hugging Face (the page's own summary; I did not read the whole post). Relevance: **a
shared board is a place coordination and collusion can emerge**. A room needs an audit trail, a human
kill switch and a rule that no member can add a channel to itself.

### 7.6 What the sources agree on

Fence peer text as untrusted data with provenance; verify the sender at the transport; keep authority
separate from content; require human approval for high-impact actions; rate-limit loops; keep an
attributable log. The label and clearance model in `DESIGN.md` section 10 is the capability separation
the sources recommend.

---

## 8. Practitioner lessons (V-PAGE where stated, otherwise REPORT or SNIPPET)

1. **Locks fail for agents, so use expiring leases and advisory claims.** Cursor (V-PAGE): "Agents
   would hold locks for too long, or forget to release them entirely. Even when locking worked
   correctly, it became a bottleneck. Twenty agents would slow down to the effective throughput of two
   or three." The fix there was roles (planners, workers, a judge) rather than a better lock.
   https://cursor.com/blog/scaling-agents
2. **Git is a workable lease store.** Anthropic's compiler experiment (V-PAGE): "Claude takes a 'lock'
   on a task by writing a text file to current_tasks/", git forces the second claimer to pick another
   task; 16 agents, nearly 2,000 sessions, $20,000. The same page notes it ran in a container with
   permission prompts skipped (REPORT). https://www.anthropic.com/research/building-c-compiler
3. **Claims help only if every agent reads them every time**, and one commenter argues for hard
   enforcement through a `PreToolUse` hook rather than advice (REPORT).
   https://dev.to/sahil_kat/coordinate-multiple-claude-code-sessions-on-a-shared-repo-1dh4
4. **Isolation does not stop semantic conflicts.** A worktree per agent merges cleanly "because Git
   compares text, not intent" (REPORT); compare plans before code.
   https://dev.to/naw103/running-multiple-claude-code-agents-in-parallel-what-worktrees-cannot-see-4le7
5. **Cost and attention are the real limit.** Token cost scales linearly with teammates (the agent teams
   page, V-DOC, which suggests 3 to 5); a practitioner reports redundant exploratory subagents burning
   tokens (SNIPPET).
6. Announce the files you will touch before touching them (a coordination skill write-up, REPORT:
   https://dev.to/lakshmisravyavedantham/i-built-a-coordination-system-for-multiple-claude-ai-agents-so-they-stop-overwriting-each-other-448i).
7. Two processes writing shared state without coordination is the classic race, fixed with worktrees
   (REPORT: https://crystl.dev/blog/agents-overwriting-files/).
8. A plain shared `board.md` with no locking or ownership is what people fall back on (Codex issue
   #21027, V-API for the issue and REPORT for its text).
9. Full-trace sharing beats message summaries for correctness (Cognition's "Don't Build Multi-Agents",
   SNIPPET).
10. Human analogues (general knowledge, UNKNOWN as sourced claims): Perforce exclusive checkout and
    git-lfs locks are server-enforced leases; CODEOWNERS is static per-path ownership; a change freeze
    is a broadcast that overrides ordinary claims (maps to the kill switch); an incident channel is one
    persistent channel with history, a roster and a human commander (maps to latecomer history and the
    human transcript); ChatOps routes actions through an audited bot (maps to routing claims through the
    gateway).

---

## 9. Discrepancies found, and how they were resolved

| # | Conflict | Resolution |
|---|---|---|
| D1 | One report put native-Windows support at v2.1.239; the first research pass at v2.1.234 | The primary page (V-DOC) says **v2.1.234** in two places. 2.1.239 is wrong |
| D2 | One report said sessions bypassing permissions hold incoming messages; the first case study observed a bypass-mode peer accepting messages | The primary page: the default (no `crossSessionInbound` value) depends on **both** sessions' mode classes. A bypassing receiver holds each message unless the sender also says it is bypassing. The observation is consistent with both sessions being in bypass mode, or with an explicit `accept` setting. Not tested separately |
| D3 | First pass: the socket's wire protocol is NOT FOUND and posting to a session is undocumented; the protocols pass: hooks and Bash can post to the socket | The primary page has a section "when you want a script or hook to post into a session": a script may post to **its own** session's socket with the documented auth line; own-child messages are delivered. The **message line format is not on the page** |
| D4 | The vendors report called issue #92060 "closed not planned" with an unknown rationale | The only comment is the stale bot; it was closed for inactivity, not decided |
| D5 | A claim that `claude-peers` push works | A different project's README says channel push does not load on a stock Claude Code 2.1.233; treat push through channels as unproven |
| D6 | Star counts and licenses in the reports | All re-read from the API; every figure matched |

## 10. What is still unknown after this survey

1. The wire format of a message line on the inbox socket, and whether a **daemon** (not a child of the
   session) can post at all, or only the session's own children (the doc's "own-child" rule suggests
   only children verify).
2. Cotal's identity model, label support and Claude Code connector behaviour (only fragments of its
   docs were read, by a research agent).
3. Whether `agent-loom`'s claims expire, and how its push behaves under `-p`.
4. The exact `LICENSE` terms for mcp_agent_mail, MCP's spec, Mattermost, Rocket.Chat and Mosquitto.
5. Whether Anthropic intends channels to leave research preview, and whether the folder-scope issue
   (#92060) has been reopened or refiled.
6. Whether any other tool already has labels or clearance. None of the ones read does; a tool not
   surveyed could.
7. Anything from forums and social sites, since none returned usable results.
