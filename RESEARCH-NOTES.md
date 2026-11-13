# Research notes: what Claude Code documents about agent-to-agent communication

Verified: 2026-11-12. Installed client: Claude Code 2.1.323. Method: documentation research by a
docs-specialist subagent (three passes), reading the official pages at code.claude.com/docs (the
`.md` form of each page, downloaded and read or searched), plus direct observation of the
cross-session tools in a live session (see `CASE-STUDY.md`). **A second pass the same day re-read the
cross-session messaging page in full myself** after survey reports disagreed with this file; it corrected
three statements (section 10) and added the details in section 11. Where section 1 and sections 10 and 11
differ, sections 10 and 11 are right.

Conventions in this file:

- **DOCUMENTED** means the statement is on a docs page I can name. The page is given after it.
- **NOT FOUND** means the researcher looked and did not find it. It does not mean it is false. It
  means it is not something this design may rely on.
- **INFERENCE** means a conclusion drawn from documented pieces, not stated by the docs.
- **OBSERVED** means seen in a real session, not read in the docs.

Coverage caveats, stated up front because they limit how far these notes can be trusted:

- Read in full: cross-session messaging, agent teams, channels, channels reference, tools
  reference, subagents, headless, remote control, and the two SDK reference pages (TypeScript,
  Python).
- Read in part (relevant sections only; the pages are very long): hooks, MCP, worktrees.
- Search-result snippets only, NOT read in full: permission modes, security.
- Linked but NOT read: the workflows page, the secure-deployment page, the costs page.
- A fetch hook in the research environment redirected some page fetches; the researcher worked
  around it by downloading the `.md` versions with curl. No page content was invented to fill a
  gap, but a page that was blocked and not re-fetched would show as "snippets only" above.
- The docs change. Re-verify the version requirements and the "research preview" status before
  building anything that depends on them.

---

## 1. Cross-session messaging

Page: https://code.claude.com/docs/en/cross-session-messaging

### What it is and what it needs

- **DOCUMENTED.** The feature is called "cross-session messaging" and has its own page.
- **DOCUMENTED.** Requires Claude Code v2.1.224 or later on macOS and Linux (including WSL 2), or
  v2.1.234 or later on native Windows. Same-machine messaging on Bedrock, Vertex or Foundry, or
  with feature-flag fetching off, needs v2.1.248 or later.
- **DOCUMENTED.** "When a session meets the requirements, messaging is on with nothing to enable."
  There is no environment variable or setting that turns it on.
- **NOT FOUND.** A GA, preview or experimental label on this page. It is gated by version and
  provider conditions, not by a flag.

### Discovery: `ListAgents`

- **DOCUMENTED.** `ListAgents` backs the `/list-agents` command (alias `/peers`).
- **DOCUMENTED.** It lists subagents, teammates, other local sessions (including background
  sessions), and, while connected to Remote Control, cloud and other-machine sessions. Source:
  https://code.claude.com/docs/en/tools-reference (tool table).
- **OBSERVED.** A listing looks like this, one row per session, with a name, a short ref in
  brackets, a kind and a status:

      Peer sessions (5):
        notes-9e [a1b2c3]      interactive  shell   started 6d ago
        service-c1 [d4e5f6]    interactive  idle    started 1d ago
        research-cb [0a9b8c]   interactive  busy    started 47m ago
        site-e2 [7d6e5f]       interactive  shell   started 1d ago
        docs-9d [3c2b1a]       interactive  idle    started 1d ago

  The calling session is named in the header ("This session is <name> [<ref>]") and is not in the
  list. Statuses seen: `busy`, `idle`, `shell`. Names are of the form `<directory-ish>-<hex>` and
  were derived from the directory the session was started in. A name changes when a session is
  restarted.
- **OBSERVED.** The tool description says a bare name works when exactly one live session matches,
  and a ` [ref]` suffix is needed only when two rows share a name.

### Sending: `SendMessage`

- **DOCUMENTED.** `SendMessage` takes a `to` target. The same tool also addresses subagents and
  teammates. Pages: cross-session-messaging and tools-reference.
- **DOCUMENTED.** `notify_when_idle`: one notice when the target next goes idle. It expires after
  12 hours.
- **OBSERVED.** Parameters: `to`, `message`, `summary` (a 5 to 10 word label for the sender's own
  transcript, not transmitted), `notify_when_idle`. The first line of `message` is what the
  receiver's human sees as a one-line preview until they expand it, so the first line should be a
  self-contained sentence.
- **OBSERVED.** A successful send reports "in that session's inbox, not yet read by its Claude".
  Success means the message reached the session, not that the model read it. For a session in a
  different permission mode a "[Cross-session delivery notice]" follows if it holds or refuses.
- **OBSERVED, from the tool description.** `@` followed by a file path in a message attaches
  nothing on the receiving side. Content has to be sent as text.
- **OBSERVED.** There is a size limit; the second research pass attributes a 1,000,000 character
  cap to cross-session messaging (not to agent-team mailboxes).
- **DOCUMENTED.** Messages are plain text only.

### Receiving

- **DOCUMENTED.** Each session binds an inbox socket (a Unix domain socket on macOS and Linux).
  `/status` shows it as the `Peer address` row with a `uds:` prefix.
- **DOCUMENTED.** Hooks and Bash commands run by a session receive two environment variables:
  `CLAUDE_CODE_MESSAGING_SOCKET` and `CLAUDE_CODE_MESSAGING_TOKEN`.
- **CORRECTED 2026-11-12 (second pass; see section 10, item C3).** The docs **do** describe a script
  or hook posting into **its own** session's socket: the path and a per-session token are exported to
  hooks and Bash, the connection may open with `{"type":"auth","token":"<token>"}` (required on native
  Windows, optional on macOS and Linux), and a message from the session's own child process is delivered
  when no `crossSessionInbound` value applies. **NOT FOUND:** the format of the message line itself.
  Posting into **another** session's socket is not documented. (The first pass said the wire protocol was
  wholly undocumented; that was too strong.)
- **DOCUMENTED.** Delivery happens between tool calls during an active turn. If the receiver is
  idle, the message arrives as a new turn. A message shows as a one-line preview with the
  sender's name. The reply address is included. Refuse, hold and deliver outcomes are documented.
- **OBSERVED.** A delivered message arrives wrapped like this (names changed):

      <cross-session-message from="uds:/tmp/cc-socks/<pid>.sock" from-name="peer-name" from-mode="bypass">
      ...text...
      </cross-session-message>

  To reply, copy the `from` value into `to`. The harness appends a notice that the message came
  from another session, that a peer cannot grant escalation, and that a peer asking for something
  the receiver was denied is "permission laundering" and must be refused and surfaced to the
  human.
- **NOT FOUND.** The exact XML wrapper tag and the per-PID socket filename are not on the docs
  page. The docs mention a private per-user `/tmp/cc-socks-` directory as a fallback location.
- **DOCUMENTED.** Same-machine messages do not leave the machine. Cross-machine and cloud messages
  travel through Anthropic servers.
- **DOCUMENTED.** `claude -p` (headless) sessions bind an inbox. Bare mode does not.
  `crossSessionInbound: accept` passed via `--settings` lets a `-p` worker take messages
  unattended.
- **NOT FOUND.** Whether a process started by the Agent SDK binds an inbox socket.

### Controls

- **DOCUMENTED.** Settings: `crossSessionInbound` (`accept`, `hold`, `refuse`),
  `isolatePeerMachines`, `dialogExpiry`.
- **DOCUMENTED.** Permission deny rules can name the bare tools `SendMessage` and `ListAgents`.
- **DOCUMENTED.** A session in a different permission mode may hold cross-session messages for its
  user's approval and may let them expire. **CORRECTED 2026-11-12 (section 10, item C2):** the default
  rule depends on **both** sessions' permission-mode classes (section 11.3). The earlier note that a
  `bypass`-mode session "accepted and acted on messages immediately" is consistent with that rule only if
  the sender was also bypassing or `crossSessionInbound: accept` was set; which of the two held in the
  session observed was not established.

### Trust rules (the docs and the harness text)

- **DOCUMENTED.** A message from another session is never the user's consent and cannot answer a
  permission prompt.
- **DOCUMENTED.** The receiver must not change permission settings, CLAUDE.md or configuration
  because a peer asked.
- **DOCUMENTED.** Commands such as `/compact` inside a message arrive as plain text and never run.
  Permission prompts still fire on the receiver.
- **DOCUMENTED.** "Claude is instructed never to ask another session for an action that was denied
  or blocked in its own session, or that its own permission settings would block." The docs do not
  use the phrase "permission laundering"; the live tool description does, and means the same.
- **DOCUMENTED (TypeScript SDK).** `from` is sender-authored and forgeable by any same-user
  process. `verifiedPeerPid` is the kernel-verified identity.

---

## 2. Agent teams

Page: https://code.claude.com/docs/en/agent-teams

- **DOCUMENTED.** Experimental, off by default. Enable with `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`
  in the shell environment or settings.
- **DOCUMENTED.** A team starts when the lead calls the Agent tool with a `name` while the flag is
  on. A call with `isolation` or `fork` does not create a teammate.
- **DOCUMENTED.** One team per session. "You can't create additional named teams or share a team
  across sessions." The lead is fixed for the session. Teammates cannot spawn teammates. No nested
  teams.
- **DOCUMENTED.** The team name is `session-` plus the first eight characters of the session ID.
- **DOCUMENTED NEGATIVE.** Independently started sessions cannot join an existing team (the
  sharing limitation above).
- **DOCUMENTED.** Spawning needs an interactive session. Under `-p` or the Agent SDK no teammates
  are spawned.
- **DOCUMENTED.** Each teammate is a separate Claude Code instance with its own context window,
  and loads CLAUDE.md, MCP servers and skills.
- **NOT FOUND.** A teammate's working directory, and whether teammates can work in different
  repositories.
- **NOT FOUND.** Worktrees for teammates. The page says only that `isolation` on an Agent call
  bypasses team creation.
- **DOCUMENTED.** "Two teammates editing the same file leads to overwrites. Break the work so each
  teammate owns a different set of files." Task claiming uses file locking. **NOT FOUND:** locking
  of file edits.
- **DOCUMENTED.** Communication is point to point: to reach everyone, "send one message per
  recipient". **NOT FOUND:** any broadcast primitive, room, channel or topic.
- **DOCUMENTED.** A shared task list exists for agents with the Task tools.
- **DOCUMENTED.** A teammate receives only its spawn prompt; the lead's history does not carry
  over. **NOT FOUND:** a late joiner reading history.
- **DOCUMENTED.** Mailboxes live at `~/.claude/teams/{team-name}/inboxes/{agent-name}.json`. The
  sending agent writes through Claude Code; a message is reported as sent only when the write to
  the recipient's file succeeds. Claude Code reads and validates each entry, reports and removes
  invalid entries, and still delivers valid ones. Teammates can read
  `~/.claude/teams/{team-name}/config.json` to discover members. The config is removed when the
  session ends; whether inboxes are removed too is an inference from the path.
- **NOT FOUND.** Mailbox ordering and size limits.
- **DOCUMENTED.** Delivery to an idle teammate is automatic; a message to an in-process teammate
  that is no longer running brings it back in the same session as its next prompt.
- **DOCUMENTED.** Size: no hard limit, 3 to 5 teammates recommended. Token cost is "significantly
  more", with a link to https://code.claude.com/docs/en/costs (not read).
- **DOCUMENTED limitations:** no resume of in-process teammates, task status can lag, slow
  shutdown, one team per session, no nested teams, no background subagents from teammates, fixed
  lead, no per-teammate permission mode at spawn, split panes need tmux or iTerm2.
- **DOCUMENTED.** Teammates start in the lead's permission mode. Teammate permission prompts
  appear in the lead's session. A teammate cannot approve a prompt on the user's behalf or relay a
  denied action to another teammate. In auto mode a classifier reviews each inter-agent message
  before delivery, treats relayed approval claims as untrusted, and a blocked message never
  reaches the recipient.
- **DOCUMENTED.** The `TeammateIdle` hook fires when a teammate is about to go idle.

**Verdict for rooms across repositories: agent teams do not fit.** A team belongs to one lead
session, cannot be joined, has no broadcast or history, and cross-repo operation is not described.

---

## 3. Channels

Pages: https://code.claude.com/docs/en/channels and
https://code.claude.com/docs/en/channels-reference

- **DOCUMENTED.** A channel is an MCP server that pushes events into a running session. It is a
  research preview. The flag syntax and protocol may change.
- **DOCUMENTED.** Events are sent with the MCP notification `notifications/claude/channel`
  (`content` plus `meta`) and arrive in the model's context as a `<channel source="...">` tag.
- **DOCUMENTED.** Subscription is opt-in per session at startup:
  `claude --channels plugin:<name>@<marketplace>`. Being listed in `.mcp.json` is not enough; the
  server must also be named in `--channels`.
- **DOCUMENTED.** Claude Code spawns the channel server as a subprocess and talks to it over
  stdio. "To process independent event streams concurrently, run separate sessions."
- **NOT FOUND.** Several sessions subscribing to one channel instance. **INFERENCE:** a hub design
  is possible: one shim per session, all connected to a shared hub. Not documented as supported.
- **DOCUMENTED.** Two-way channels expose a `reply` tool (server declares `capabilities.tools`,
  implements ListTools and CallTool, and sets `instructions`). The terminal shows only the tool
  call and confirmation, not the reply text.
- **NOT FOUND.** Routing a reply to a specific session.
- **DOCUMENTED.** Events queue and are processed in order; several queued events are handled
  together on the next turn. Events arrive only while the session is open.
- **NOT FOUND.** An explicit statement that an idle session wakes on a channel event. The
  quickstart shows Claude acting on an arriving message, which is not the same statement.
- **DOCUMENTED.** Non-interactive: channels can run under `-p` and tools that need terminal input
  are disabled. **BUT** custom channels need `--dangerously-load-development-channels` during the
  research preview, and that flag is **ignored under `-p` and the Agent SDK**. Whether an approved
  channel registers under `-p` with no further caveat: NOT FOUND.
- **DOCUMENTED.** To build one: declare `capabilities.experimental['claude/channel']: {}`, emit
  `notifications/claude/channel`, use stdio. Register with an `mcpServers` entry in `.mcp.json` or
  `~/.claude.json`. Wrap as a plugin, publish to a marketplace, enable with `--channels`. A
  marketplace channel still needs the development flag unless it is on the allowlist.
- **NOT FOUND.** A self-service path onto the default allowlist. The docs point to an Anthropic
  partner contact, or an org admin adding it to `allowedChannelPlugins`.
- **DOCUMENTED.** Admin controls: `channelsEnabled` (master switch) and `allowedChannelPlugins`.
- **DOCUMENTED.** Permission relay: a server may declare
  `capabilities.experimental['claude/channel/permission']: {}`; Claude Code sends
  `notifications/claude/channel/permission_request` and the server relays the verdict. It covers
  tool-use approvals only, not project-trust or MCP consent dialogs.
- **DOCUMENTED.** Sender gating: the channel must check the sender's identity (`message.from.id`),
  not the room, against an allowlist before emitting.
- **DOCUMENTED.** Requirements: Anthropic authentication (claude.ai or Console key). Not available
  on Bedrock, Google Cloud's Agent Platform or Microsoft Foundry. Team and Enterprise
  organizations must enable channels. Bun is required for the official plugins; custom channels
  can use Node or Deno.
- **DOCUMENTED.** On the v2 MCP client runtime, Claude Code does not register a channel server
  that negotiates protocol revision 2026-07-28.
- **NOT FOUND.** A version number for the channels feature.

---

## 4. Hooks

Page: https://code.claude.com/docs/en/hooks (read in part)

- **DOCUMENTED.** Events include `UserPromptSubmit`, `PostToolUse`, `SessionStart`, `Stop`,
  `SubagentStart`, `PostModelSwitch`, `TeammateIdle`, `Notification`.
- **DOCUMENTED.** For `UserPromptSubmit`, plain stdout and `additionalContext` are each injected as
  a system reminder that Claude reads. `additionalContext` is wrapped in a system reminder,
  inserted where the hook fired, and read on the next model request.
- **DOCUMENTED.** `hookSpecificOutput.additionalContext` is listed for `SessionStart`,
  `SubagentStart` and `PostModelSwitch` (context only, no blocking), and for `Stop` and
  `SubagentStop` (non-error feedback that continues the conversation).
- **DOCUMENTED WITH CAVEAT.** `PostToolUse` is named in the "Add context" section as a mid-session
  event where context is injected, but the per-event field table does not list `additionalContext`
  for it. **OBSERVED** in a live session: hook output on PostToolUse reached the model as
  additional context.
- **DOCUMENTED.** Each of `additionalContext`, `systemMessage`, `initialUserMessage` and plain
  stdout is capped at 10,000 characters.
- **NOT FOUND.** A general statement that hooks run while a session is idle; lifecycle hooks fire
  at points in the loop. Two documented exceptions: the `Notification` hook with `idle_prompt`
  fires about 60 seconds after Claude finishes responding if the user has not typed and no
  background agent is running (observational, no decision control); async hooks deliver output on
  the next turn, and an `asyncRewake` hook that exits with code 2 wakes Claude immediately even
  when idle. The `asyncRewake` timeout still applies.
- **INFERENCE.** Using `asyncRewake` to wait for an external message is not a documented pattern.
- **DOCUMENTED.** Under `-p`, async hooks still running at teardown are killed.
- **NOT FOUND.** A hook event for an inbound channel event or an inbound peer message.

## 5. MCP

Page: https://code.claude.com/docs/en/mcp (read in part)

- **DOCUMENTED.** Server-initiated `list_changed` notifications for tools, prompts and resources:
  interactive sessions refetch the list; under `-p` and the SDK only the tool list refreshes.
- **NOT FOUND.** Arbitrary server notifications surfaced to the model. The only documented path
  that pushes server content into the model's context is the channel mechanism.
- **NOT FOUND.** Sampling (`sampling/createMessage`) and resource subscriptions
  (`resources/subscribe`).
- **DOCUMENTED.** Roots (`roots/list`, `notifications/roots/list_changed`), elicitation
  (interactive terminal sessions only), and WebSocket servers that can push events unprompted.

## 6. Worktrees

Page: https://code.claude.com/docs/en/worktrees (read in part)

- **DOCUMENTED.** A worktree has its own files and branch and shares history and remote with the
  main checkout. Default location `.claude/worktrees/<name>/`, branch `worktree-<name>`.
- **DOCUMENTED.** Giving each session its own worktree "gives it a separate copy of the files to
  edit", so changes do not collide. Shared with the main checkout: the `.git` directory,
  project-scope plugins, permission approvals saved to the main checkout's
  `.claude/settings.local.json`, and untracked skills or agents when the worktree has none.
- **DOCUMENTED.** Running agents hold a `git worktree lock`. Subagents can run in worktrees via
  `isolation: worktree`.
- **DOCUMENTED.** Cross-session messaging is named as the way to "coordinate parallel worktrees".
- **NOT FOUND.** Worktrees across different repositories. Two sessions editing one checkout without
  a worktree: no explicit guidance, the docs recommend worktrees.

## 7. The Agent SDK (TypeScript and Python)

Pages: https://code.claude.com/docs/en/agent-sdk/typescript, /python, /subagents

- **DOCUMENTED.** Subagents via the `agents` option and `AgentDefinition` (description, prompt,
  tools, disallowedTools, model, mcpServers, skills, maxTurns, memory, effort, permissionMode;
  Python adds `background`).
- **DOCUMENTED.** Sessions: `resume`, `continue`, `session_id` (Python), `forkSession` /
  `fork_session`, `resume_session_at` (Python); helpers to list, read, fork, rename and tag
  sessions.
- **DOCUMENTED.** Input and clients: `query()`, `ClaudeSDKClient` (Python, multi-turn),
  `streamInput` (TypeScript), async-iterable prompts. Permissions: `canUseTool` / `can_use_tool`,
  `allowedTools`, `disallowedTools`, `permissionMode`. Hooks and `mcpServers` options. Hook events
  include `SubagentStart`, `TeammateIdle`, `TaskCreated`, `TaskCompleted`.
- **DOCUMENTED.** The TypeScript reference defines a `peer` message origin (`from`, `fromMode`,
  `name`, `body`, `verifiedPeerPid`): peer messages from teammates or cross-session peers.
- **NOT FOUND.** Any SDK function equivalent to `SendMessage` or `ListAgents`. Any SDK API to send
  into a running interactive session. A statement on whether SDK-spawned processes bind an inbox.

## 8. Other pieces

- **Headless resume.** `claude -p ... --continue` or `--resume <session_id>`;
  `--output-format json` returns `session_id`. Sequential turns, no live delivery.
  https://code.claude.com/docs/en/headless
- **Remote Control.** A human-facing bridge to claude.ai/code and the Claude app. Needs a
  claude.ai sign-in, does not accept API keys, carries cross-session messages across machines. No
  programmatic send API found. https://code.claude.com/docs/en/remote-control
- **Workflow tool.** A script that orchestrates many subagents in the background and returns one
  result. Not read in detail.
- **Permission modes (snippets only).** Six: `default`, `acceptEdits`, `plan`, `auto`, `dontAsk`,
  `bypassPermissions`. Deny rules apply in every mode including bypass. Subagents generally
  inherit the parent's mode. https://code.claude.com/docs/en/permission-modes
- **Security (snippets only).** Defenses are the permission system, command-injection detection and
  network approval; `bypassPermissions` gives no protection against prompt injection; auto mode's
  classifier can block actions driven by hostile content.
  https://code.claude.com/docs/en/security
- **NOT FOUND.** An Anthropic-documented mailbox or message-bus MCP pattern. A dedicated section on
  prompt injection across agents or sessions (closest: the per-message rules above and the
  secure-deployment page, linked and not read).

## 9. Summary table: which piece can carry a room

| Need | Cross-session messaging | Agent teams | Channels | Hooks | MCP |
|---|---|---|---|---|---|
| Independent sessions, different repos | yes (same machine) | no | per session only | per session | per session |
| Many-to-many room | no | within one team only | not documented | no | no |
| Push to a busy session | between tool calls | yes within a team | queued, next turn | at hook points | list changes only |
| Push to an idle session | new turn | resumes teammate | not explicit | `asyncRewake` (inference) | no |
| History for a late joiner | no | no | no | no | no |
| Works under `claude -p` | yes (binds inbox) | no | custom channels: no | yes (async killed at teardown) | yes |
| Usable without a research-preview flag | yes | no (experimental) | no for custom | yes | yes |

---

## 10. Corrections made by the second pass (2026-11-12)

Three statements in the first version of this file were wrong or too strong. They were found when three
survey reports disagreed with it and the primary page was re-read in full
(https://code.claude.com/docs/en/cross-session-messaging, fetched as Markdown, 36,437 characters).

| # | First-pass statement | What the primary page says | Where fixed |
|---|---|---|---|
| C1 | Native Windows needs v2.1.234 | **Confirmed**, stated twice on the page. A survey report said v2.1.239; that was wrong | no change needed |
| C2 | A `bypass`-mode session "accepted messages immediately" (read as: bypass receivers accept) | The default is decided from both sessions' mode classes: a bypassing receiver **holds** each message unless the sender also says it is bypassing (section 11.3) | section 1, Controls |
| C3 | The inbox socket's wire protocol is NOT FOUND; posting into a session is undocumented | The page has a section "when you want a script or hook to post into a session": posting into the session's **own** socket is documented, with an auth line and own-child verification; only the **message line format** is missing | section 1, Receiving |

What was **not** wrong: the existence and names of the settings, the 12-hour `notify_when_idle` expiry,
that `-p` sessions bind an inbox and bare mode does not, and the rule that a peer's message is never the
human's consent.

---

## 11. Details verified on the primary cross-session page (second pass)

All items are **DOCUMENTED** on https://code.claude.com/docs/en/cross-session-messaging unless marked.

### 11.1 Versions and availability

- Requires Claude Code v2.1.224 or later on macOS and Linux (including WSL 2); v2.1.234 or later on
  native Windows.
- Messaging with a session on **another** of your machines requires v2.1.225 or later and a target that
  appears in the listing.
- The `/config` row **Messages from your other sessions** (sets `crossSessionInbound`) requires v2.1.232.
- The transcript's `Waiting to be delivered` list for a message a busy session has not yet received
  requires v2.1.309.
- Same-machine messaging is available on every provider (Bedrock, Claude Platform on AWS, Google Cloud's
  Agent Platform, Foundry) and with feature-flag fetching off.
- The changelog shows no later entry about messaging, channels or teams after 2.1.323 (REPORT; the
  changelog was read by a survey agent).

### 11.2 Delivery and limits

- The receiving Claude reads a message **between tool calls** during an active turn. An idle session
  **starts a new turn** with it. Delivered messages count toward usage like a typed prompt.
- A message is plain text. An `@` file or MCP mention is shown as written and attaches nothing.
- Refused before sending: a message over about **one million characters** (serialized); a rapid burst to
  one session, once it reaches what that session's inbox accepts (the refusal tells Claude to batch or
  wait); a target that is not listed as able to receive; a reply target that fails a safety check such as a
  symlink.
- At the receiver: repeated messages per sender are rate-limited, **identical repeats within a short window
  are dropped**, and **at most 50 accepted messages are queued** for Claude to read. "A message loop
  between two sessions therefore stops on its own."
- A receiving check ends in one of three outcomes: **delivered**, **held** (set aside; reaches Claude only
  on approval or a later mode or settings change), **refused** (dropped).
- The receiver can reply with `SendMessage`, except for one-way cross-machine messages and when messaging
  is off.
- The receiving human sees a dim one-line preview, for example `› Message from @api-worker: Schema
  migration finished (ctrl+o to expand)`, containing the sender's name and the first line, cut with an
  ellipsis; Ctrl+O shows the full text; `--verbose` shows the full text inline. **Claude always reads the
  full message.**
- Claude receives the message **with the sender's name and a reply address** (except one-way messages).
- `notify_when_idle`: one notice when a target next goes idle or exits, "idle" meaning a finished turn
  with nothing queued; the watched session can drop the request without answering, in which case the
  subscription expires unanswered after 12 hours.

### 11.3 The inbound controls and the default hold rule

- `crossSessionInbound` values: `accept` (deliver each message), `hold` (show a notice, do not deliver;
  released if an `accept` later applies), `refuse` (drop each message). From project or local settings,
  `refuse` applies over every other source; from user settings it applies unless managed settings or
  `--settings` set a value.
- When **no value applies**, Claude Code decides per message from the two sessions' permission modes. It
  puts sessions that **bypass permission prompts** into one class and all others into the other class. Plan
  mode counts as bypassing in interactive terminal sessions that have bypass permissions available; `auto`,
  `acceptEdits` and `dontAsk` count as prompting.
  - **Receiver prompts for permissions:** each message is delivered, **except** a message is held for
    approval when the sender identifies itself as bypassing.
  - **Receiver bypasses permission prompts:** each message is **held** for approval, **except** a message
    is delivered when the sender also identifies itself as bypassing.
- A held message in an interactive terminal opens an approval dialog showing the sender and a preview;
  approve delivers that one message; deny or dismissing drops it; unanswered past `dialogExpiry` (default
  **five minutes**) it is closed and the message dropped. For a background session with no terminal attached
  the dialog stays open past the deadline until attached. If the session's mode class changes while messages
  are held, the rules are re-applied and now-accepted messages are delivered.
- At most **100** held messages; past that the oldest are dropped.
- **`-p` sessions bind an inbox** like interactive ones and appear in the listing. They cannot show the
  dialog, so a default-held message is kept for the same `dialogExpiry` deadline, delivered if a mode or
  settings change allows it, otherwise dropped and reported expired to a reachable sender. `dialogExpiry:
  "never"` keeps default-held messages until the session ends. A message held by an **explicit** `hold`
  setting does not expire. Bare mode does not bind the socket.
- Sessions in the VS Code extension or the Desktop app cannot show the dialog and keep a held message until
  the same deadline.

### 11.4 The session's inbox socket

- Each session with messaging enabled binds an inbox socket: a Unix domain socket on macOS and Linux
  (including WSL 2), a named pipe on native Windows. `/status` shows it in the `Peer address` row with a
  `uds:` prefix.
- Claude Code exports it to hooks and Bash commands as `CLAUDE_CODE_MESSAGING_SOCKET`, **before any hook
  runs, including `SessionStart`**, in a session that starts with messaging on.
- On macOS and Linux the socket is restricted to your operating-system user; on native Windows each
  connection must authenticate first with a key readable only by your user. If the preferred directory is not
  acceptable (for example owned by another user), it uses `/tmp/cc-socks-<uid>`. If no directory is
  acceptable the session runs **without an inbox**, `/status` shows `unavailable` and the reason, and the
  `--debug` log has the refusal.
- A per-session token is exported as `CLAUDE_CODE_MESSAGING_TOKEN`. A script posting to its **own**
  session's socket can send `{"type":"auth","token":"<token>"}` as the first line of its connection. On
  macOS and Linux the line is optional; on native Windows it is required and any connection whose first
  line is not a valid auth line is closed with nothing delivered.
- "Open the connection only when the message you're posting is ready." A connection that has not sent a
  complete line within **30 seconds** is closed, so capture slow output first.
- Messages arriving on the socket go through the same inbound controls as any peer message, with two
  conditions:
  - **Own-child messages:** when no `crossSessionInbound` value applies, a message verified to come from
    the session's own child processes (a hook or Bash command posting back to its own session) is
    **delivered**. On Linux, process evidence verifies even a child that has exited. On macOS it verifies
    only while the posting process is still running; after it exits (and in a container where Claude Code
    runs as process ID 1, and on native Windows) the child must send the token in the auth line. If
    neither verifies, the message is treated like any other that asserts no permission class, so a session
    that bypasses permission prompts **holds** it.
  - **Sandboxed sessions:** whether a Bash command can reach the socket is controlled by
    `sandbox.network.allowAllUnixSockets` and `sandbox.network.allowUnixSockets`.
- **NOT FOUND on this page:** the format of the message line after the auth line.

### 11.5 Restricting and trust

- `isolatePeerMachines: true` requires explicit approval before any `SendMessage` reaches a session beyond
  this machine, even in bypass mode; a `true` from any settings scope applies; same-machine messages never
  prompt.
- Turn off receiving with `crossSessionInbound: refuse`; turn off sending and listing with permission deny
  rules naming `SendMessage` and `ListAgents`; administrators can do both in managed settings.
- A message "can't approve anything" (never counts as consent), "can't change configuration" (the receiving
  Claude is told never to change permission settings, `CLAUDE.md` or other configuration because another
  session asked), "commands don't run" (`/compact` arrives as plain text), and permission prompts still fire.
- Claude is instructed never to ask another session for an action that was denied or blocked in its own
  session, or that its own settings would block, and to route that work back to the human.
- On the listing: while connected to Remote Control, Claude Code withholds some details of local sessions
  from `/list-agents` output, including names it cannot attribute to a person, shown as `(unnamed session)`.

### 11.6 Identity as the docs present it

- A session is addressed by **name**. Names come from `/rename`, `--name` or auto-assignment. The `@`
  typeahead lists live sessions on the machine; a name with a space or other characters outside letters,
  digits, hyphens and underscores must be quoted, as in `@"release notes"`. When more than one live session
  answers to a mentioned name, Claude asks which is meant. Two same-named sessions in different directories
  get a short identifier added in the listing.
- **A name is a label, not a credential.** Nothing on the page binds a name to a verified identity beyond
  the operating-system user who owns the socket.

---

## 12. Note on the summary table in section 9

Section 9's "Push to an idle session" row lists `asyncRewake` for hooks as an inference. The second pass
added a documented mechanism not in that table: a hook or background process **the session started** can
post to **its own** inbox socket (section 11.4), which Claude Code delivers between tool calls or as a new
turn when idle, subject to the inbound rules in section 11.3. Its line format is not documented, so it is a
candidate, not a supported pattern. See `DESIGN.md` section 9.

---

## 13. Third pass (2026-11-12): the inbox socket in practice, Codex CLI, and Jev

Tags as before, plus **REPORT** (a third-party write-up, not verified against a primary source) and
**SNIPPET** (seen only in a search-result summary, not read in full). Facts here are as of 2026-11-12.

### 13.1 Claude Code inbox socket, re-read in full

- **DOCUMENTED** (re-read on the primary page): the socket is restricted to the OS user; the fallback directory
  is `/tmp/cc-socks-<uid>`; `CLAUDE_CODE_MESSAGING_SOCKET` is exported before any hook runs, including
  `SessionStart`; an own-child message is delivered by default when no `crossSessionInbound` value applies;
  Linux verifies a child by process evidence even after it exits, macOS only while it is still running, and a
  container where Claude Code is process 1 and native Windows need the token in the auth line; an unverifiable
  message counts as asserting no permission class, which a bypassing session holds.
- **DOCUMENTED:** limits are plain text, about one million characters on the same machine, a sender-side
  refusal on a rapid burst, and receiver-side rate limits, drop of identical repeats and a queue of 50.
  Versions: 2.1.224 (macOS, Linux, WSL 2), 2.1.234 (native Windows), 2.1.232 for the `/config` row and `@`
  mentions, 2.1.248 for same-machine messaging on third-party providers or with feature-flag fetching off.
- **NOT FOUND (still):** the format of the message line after the auth line, any acknowledgement, and whether a
  same-user process that is not a child is a supported sender. Anthropic issue 103123 asks for exactly this; it
  was open when read, labelled docs and enhancement, with no maintainer reply in what was retrieved.
- **OBSERVED by others (issues 103123 and 93720, open when read):** on 2.1.302 (macOS) a plain-text line is
  silently ignored; the working example is `{"type":"user","message":{"role":"user","content":"..."}}` after the
  auth line; a `from` field is ignored.
- **OBSERVED by others, correcting section 11 and an earlier summary:** a raw post renders **differently by
  timing**. To an idle session it renders as a full user turn with the header "Another Claude session sent a
  message:", the body and a roughly 120-word permission-laundering paragraph, with no collapsed preview. Posted
  mid-turn it renders the body alone, without the envelope. A real `SendMessage` renders as
  `› Message from @name: ... (ctrl+o to expand)`. (Earlier text said a raw post is always bare text; that was
  true only mid-turn.) Reproduced on 2.1.268 and 2.1.270.
- **SNIPPET (not read in full):** the socket path is listed in `~/.claude/sessions/<pid>.json` and a token file
  sits beside it with 0600 permissions; every resume or restart changes the process id and invalidates peer
  names and reply addresses (issue 89338); some sessions register but never bind the socket until restarted
  (issue 85497); messaging works per Unix account only (a third-party issue); a gist reports a 16 KiB limit,
  which conflicts with the documented million characters and is treated as unknown.
- **INFERENCE:** a daemon that is not the session's child is the case the docs do not describe. By the default
  rule it asserts no class, so a prompting receiver would deliver it and a bypassing receiver would hold it.
  Untested. The design avoids it (`DESIGN.md` D9).

### 13.2 Codex CLI (OpenAI), for the harness abstraction

- **DOCUMENTED** (OpenAI hooks page, read in full): Codex has command hooks. Events include `SessionStart` (source
  `startup`, `resume`, `clear`, `compact` or `fork`), `UserPromptSubmit`, `PreToolUse`, `PostToolUse`,
  `PermissionRequest`, `Stop`, `SubagentStart`, `SubagentStop`, `PreCompact`, `PostCompact` and `SessionEnd`.
  Plain stdout from `SessionStart` and `UserPromptSubmit` is added as developer context.
- **DOCUMENTED:** `PreToolUse` can intercept shell commands, `apply_patch` file edits (matchers `apply_patch`,
  `Edit` or `Write`), MCP tool calls and other local function tools. It denies by a JSON decision or exit code 2
  and may add context or rewrite input. Hosted tools such as web search are not covered. The page itself says
  to treat tool hooks as "a useful guardrail, not a complete enforcement boundary".
- **DOCUMENTED:** every hook receives `session_id`, `transcript_path`, `cwd`, `hook_event_name` and `model` on
  stdin; most events also receive `permission_mode` (`default`, `acceptEdits`, `plan`, `dontAsk`,
  `bypassPermissions`). The transcript format is not a stable interface. A `Stop` hook returning
  `decision: "block"` makes Codex continue with the reason as a new prompt.
- **DOCUMENTED:** a non-managed hook must be reviewed and trusted by hash (`/hooks`); a new or changed hook is
  skipped until trusted. Managed hooks come from `requirements.toml`, MDM or cloud sources and are trusted by
  policy. `--dangerously-bypass-hook-trust` exists.
- **REPORT** (third-party guide; check OpenAI's docs): Codex CLI v0.149.0 added `codex queue --session <id or
  name> "text"`, which appends to a running session's input queue, wakes an idle session and holds the
  message until the turn ends if the session is busy; the queued message inherits the session's permission
  profile; any process able to reach the local socket or the remote WebSocket can inject a user turn.
- **SNIPPET and REPORT:** a proposal for local cross-session messaging over a Unix socket (openai/codex issue
  16447) was open when read, with comments recommending a stable id instead of a display name and an explicit
  acknowledgement. Whether `codex queue` supersedes it was not checked. Other tools reported: `agmsg` (a shared
  SQLite database across several vendors' CLIs), `repowire` (a local daemon with `list_peers` and
  `notify_peer`) and `agent-talk` (a web relay).
- **REPORT:** Codex acts as an MCP client (`mcp_servers` in `~/.codex/config.toml`); it reads `AGENTS.md` files
  from the project root down to the working directory, up to 32 KiB; `codex exec` runs headless, with a read-only
  sandbox and no approval prompts by default; sandbox mode and approval policy are separate settings. Sources
  disagree on defaults and some are out of date.

### 13.3 Jev (TypeSafe AI), for the optional judge

- **REPORT:** Jev is a hosted decision model, released 2026-09-15 by TypeSafe AI, that takes a state and typed
  questions and returns typed answers with probabilities rather than text. The vendor claims large speed and
  cost advantages over LLMs on classification; these are vendor figures. It is used as a gate in front of agent
  actions, with a stronger model for low-confidence cases. A paper reports a cascade with fewer strong-model
  calls and other evaluations report limited advantage over a cheap cascade.
- **REPORT:** it cannot explain its answers and cannot verify what is not in its input, and an industry article
  warns that the text it reads can influence its verdict. It is a hosted API, so text sent to it leaves the
  machine (`DESIGN.md` D4: hosted judges are label-gated and optional). An open-source Jev-like model, Kev, was
  named in a gate paper; its license and local use were not checked (`DESIGN.md` question 19).

### 13.4 Sources

Anthropic: the cross-session messaging page; issues 103123 and 93720 (read); issues 85497 and 89338 (snippets).
OpenAI: the Codex hooks page (read); issue 16447 and a `codex queue` article (read). Jev: LangChain, AWS builder,
Harness and VentureBeat write-ups and an arXiv gate paper (snippets only).

## 14. Experiments run on 2026-11-12 (Claude Code 2.1.323, macOS, Darwin 23.3.0)

Method: throwaway `--settings` files with `--setting-sources local` (so none of the user's own hooks loaded),
scratch working directory, headless workers (`claude -p`, and `-p` with stream-json input to keep a worker idle
between turns). The user's real login was used because an isolated `CLAUDE_CONFIG_DIR` starts logged out and
would need an interactive `/login`. Side effect: each worker wrote a transcript under the user's
`~/.claude/projects` for the scratch directory. Every hook and probe wrote only inside the scratch directory.
Each result is **OBSERVED** on one version on one machine; none is a guarantee.

### 14.1 Hooks (DESIGN section 14 questions 1, 2, 3, 5 and 23)

| Question | Result |
|---|---|
| Does hook stdout reach the model? | **Yes** for `SessionStart` and `UserPromptSubmit`: the model quoted both marker lines. `PostToolUse` was not tested. |
| Do hooks run in `claude -p`? | **Yes**, including `SessionStart`, `UserPromptSubmit` and `PreToolUse`. |
| What does a hook receive? | JSON on stdin: `session_id`, `transcript_path`, `cwd`, `hook_event_name`. `UserPromptSubmit` and `PreToolUse` also carry `permission_mode`. `SessionStart` carried `source` and no `permission_mode`. `PreToolUse` carries `tool_name`, `tool_input` (for Bash, the full `command`) and `tool_use_id`. |
| Can `PreToolUse` see and block a Bash call? | **Yes.** Exit code 2 blocked the call even under `bypassPermissions`. The model received the hook's stderr text as a `PreToolUse:Bash hook error` and did not retry. Warn-without-block and output limits were not tested. |
| Does `asyncRewake` wake an idle session? | **Yes** in a headless stream-json session: a `SessionStart` hook with `async` and `asyncRewake` that slept 15 seconds and exited 2 caused a new turn, and the model acted on the stderr text. Not tested in the interactive TUI or with long delays. |

### 14.2 Sockets (DESIGN section 14 questions 12 and 13)

- **A headless worker registers like an interactive session.** `claude -p` with stream-json input wrote
  `~/.claude/sessions/<pid>.json` (kind `interactive`, entrypoint `sdk-cli`, status `idle`), a key file and a
  socket under `/tmp/cc-socks/`. Registry fields seen: `pid`, `sessionId`, `cwd`, `startedAt`, `procStart`,
  `version`, `peerProtocol`, `peerFeatures` (`notify_idle`, `reply_across_default_dirs`, `artifact_yield`),
  `kind`, `entrypoint`, `pidDomain`, `messagingSocketPath`, `name`, `nameSource`, `status`, `updatedAt`. The
  key file holds `peerToken`, `procStart` and `pidDomain`.
- **A process that is not the session's child delivered a message.** An auth line plus
  `{"type":"user","message":{"role":"user","content":"..."},"priority":"now","uuid":"..."}` was accepted by an
  idle worker in `default` mode and the model replied. It saw the text as an ordinary user message with no
  sender. No acknowledgement was returned on the socket.
- **The token was not enforced in this run.** A wrong token and a missing auth line were both delivered
  (three posts sent in quick succession; the model acted on all three). The effective gate looked like the
  socket's `0600` mode, which means same-user processes can post without reading any key file. Do not rely on
  this; it contradicts third-party descriptions and may be version-specific. Klatch should not depend on the
  token or the socket mode for its own security.
- **Not tested:** a busy session, the interactive TUI, a bypassing receiver, and the hold and refuse paths.

### 14.3 Registering a fake peer (DESIGN section 22, new questions 27 and 28)

A throwaway process owned a socket, wrote a registry record and a key file modelled on the real ones
(second attempt: valid UUID session id, key file named by the SHA-256 of the token), and a `bypassPermissions`
worker was told to run `ListAgents` and `SendMessage` to it. Both attempts: **the record was not listed** and
`SendMessage` reported the name as unreachable. In the second attempt a process connected to the fake socket
twice with an empty payload before the record was rejected (that process was most likely the worker checking
liveness; its pid was recorded but not matched to the worker). The server obtained the peer pid with
`LOCAL_PEERPID` (`getsockopt(0, 2)` on macOS). Conclusion: a hand-built record is not enough, probably because
Claude Code checks more than the file (process evidence or a handshake). **Native-peer mode is not available by
simple forgery**, and no further attempt was made. Registry and sockets were cleaned up afterwards.

### 14.4 What this changes

- Posting from outside to a session that is already running works today with no change to that session, so
  Tier 0 nudging is feasible; it stays undocumented and version-probed, and content-free.
- The shipped hook path (Tier 1) is confirmed for delivery, enforcement and wake-up in headless workers.
- Native-peer mode (registering the gateway as a peer) stays out of v1; the default remains the CLI, hooks and
  the content-free nudge.
