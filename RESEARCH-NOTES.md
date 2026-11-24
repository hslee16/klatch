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

## 15. Other transports, MCP, ACP and A2A (2026-11-12)

Mostly third-party summaries found by web search; the MCP specification and the Codex configuration reference are
the primary sources, and only their search snippets were read. Verify before relying on a detail.

- **MCP.** **REPORT:** the current revision is 2026-07-28. Transports are stdio and Streamable HTTP; HTTP+SSE is
  deprecated. In that revision the initialize handshake and protocol sessions are removed, server-initiated
  requests are replaced by `input_required` results, and server push uses `subscriptions/listen` (a client lists
  resource URIs and receives `notifications/resources/updated`). Tasks moved to an extension; roots, sampling and
  logging are deprecated. Authorization on HTTP is an OAuth 2.1 profile.
- **MCP in the harnesses.** Claude Code handles `list_changed` for tools, resources and prompts and refreshes
  only the tool list in `-p` mode; per-resource subscribe support is marked unknown. A remote server added as a
  claude.ai connector reportedly keeps its start-of-session tool list (issue 108596). Codex reads MCP servers from
  `config.toml` (stdio and Streamable HTTP, OAuth login, tool allowlists); one source says only the tools part of
  the spec is implemented (**unverified**). A stateless HTTP MCP server cannot push at all.
- **ACP (Agent Client Protocol).** An editor starts an agent as a subprocess and exchanges JSON-RPC over stdio;
  remote transport is still in development. Zed and JetBrains support it natively; Claude Code and Codex are
  reached through adapters (`codex-acp`, a Claude Code SDK wrapper). Relevance: a user-facing klatch agent such as
  the librarian could be offered to editors through it.
- **A2A v1.0 (March 2026).** Agent to agent. Data model in Protocol Buffers with JSON-RPC, gRPC and HTTP+JSON
  bindings; discovery by an Agent Card at `/.well-known/agent-card.json`. Task-oriented between independent
  agents, unlike many-to-many rooms; IBM's ACP merged into A2A in August 2025. A bridge is possible later.
- **Generic transports.** WebSocket: duplex, simplest for UI shims. gRPC: typed bidirectional streams, good for
  server-to-server. NATS and MQTT: brokered pub/sub (a broker was rejected for same-machine use; MQTT targets
  constrained devices). QUIC and WebTransport: promising, less mature. Tailscale or WireGuard is a network layer
  and the practical way to reach another machine (not researched here).
- **What it changes.** `DESIGN.md` section 20 rules 12 and 13: every transport is duplex, and MCP is an adapter in
  v1 and an optional connector later, not the core transport.

Sources: modelcontextprotocol.io (specification 2026-07-28, resources); code.claude.com/docs/en/mcp;
canimcp.dev (Claude Code feature support); developers.openai.com/codex/config-reference; zed.dev/blog/claude-code-via-acp;
restfulapi.net and getmaxim.ai (MCP, A2A and ACP comparisons); IETF draft on messaging for agentic AI;
pilotprotocol.network (NATS vs gRPC).

## 16. Software factories and agent orchestrators (2026-11-12)

Web search, mostly third-party and vendor material; verify details before relying on them.

- **Shape.** **REPORT:** a software factory is the system around coding agents: an intake queue, isolated
  workspaces (git worktrees or containers), a shared tool layer, automated verification and an explicit merge
  gate. Factory's own wording is "signal in, software out, humans govern". "Dark factory" (a term attributed to
  Dan Shapiro, January 2026) is the level where nobody reads the diff.
- **Platforms and tools named.** Factory (Droids, Missions), Warp Factories, 8090, GitLab Duo Agent Platform;
  orchestrators Orchestrator.inc (Apache-2.0, an orchestrator agent spawns workers in worktrees and routes CI and
  review feedback back to the owning agent), Claude Squad (AGPL-3.0), Conductor, dmux, Agentastic (50 or more
  parallel agents), Agent Swarm, Open Multi-Agent (task DAG), and Claude Code's experimental agent teams.
  General frameworks (LangGraph, CrewAI, OpenAI Agents SDK, Microsoft Agent Framework) build agents rather than
  wrapping coding CLIs. No single "agent factory" library was found.
- **Patterns relevant to klatch.** One source says the handoff, not the conversation, is the unit of
  coordination. Parallel agents in isolation can still invent incompatible API contracts (the shared-artifact
  problem claims address). Separating the developer agent from a verification agent is recommended. Spotify,
  Cloudflare, Stripe and Faire are cited as in-house factories; one paper reports a 15x rise in daily merges and
  at least 5.6% of issues marked fixed later found wrongly fixed (an association, not a cause).
- **What it changes.** `DESIGN.md` section 20 rule 14 and a new non-goal: klatch is not an orchestrator,
  scheduler, workflow engine or merge gate. Open work is the epic "Software factory integration".

Sources: github.com/varun1505/awesome-software-factories; mastra.ai and firecrawl.dev software factory articles;
factory.ai/product/software-factory; arxiv.org/abs/2610.36254; github.com/OrchestratorInc/agent-orchestrator;
github.com/andyrewlee/awesome-agent-orchestrators; augmentcode.com orchestrator roundup;
hackersandwizards.dev multi-agent orchestration notes.

## 17. Discovering and identifying running sessions (2026-11-12)

Observed in this Claude Code session (2.1.323, macOS), one observation each, read-only.

- **Environment of the Bash tool.** `CLAUDE_CODE_SESSION_ID` is set, and so are `CLAUDE_PID`, `CLAUDE_CODE_ENTRYPOINT`,
  `CLAUDECODE` and the session's own `CLAUDE_CODE_MESSAGING_SOCKET` and `CLAUDE_CODE_MESSAGING_TOKEN`. This answers
  question 6 for a plain session; forks and sub-sessions are still untested. The messaging token is in the child
  environment, so a klatch process must never log or print its environment.
- **Process ancestry.** Walking up the parent chain from the Bash tool's shell reached the `claude` process whose pid
  has a registry record, and the record's session id matched `CLAUDE_CODE_SESSION_ID`.
- **The registry** (`~/.claude/sessions/<pid>.json`) lists every live session of the user with pid, session id, working
  directory, name and status; see section 14.2 for the full field list. Reading it needs no key file.
- **What it supports.** `DESIGN.md` section 20 rule 15: read-only discovery, invites by content-free nudge, identity
  from process ancestry. Not tested: resume and fork (pid changes on resume, question 26), other harnesses, and
  Linux.
- **Liveness of registry records.** Checked for all five records on this machine, read-only. Every record's pid was
  alive and its `procStart` equalled `ps -o lstart=` run with `TZ=UTC` (format of `ps`, with a padded day such as
  `Nov  6`); compared in local time (PST) every record looked mismatched, so a naive comparison would call live
  sessions stale. `updatedAt` ages ranged from one minute to over four hours for live sessions, so freshness is not
  a liveness test. The `pidDomain` field (`darwin` here) marks which pid space the record belongs to. Not tested:
  a dead pid, a reused pid, and Linux (where start time would come from `/proc`).
- **Linux (2026-11-18, #171).** Still **not observed**: no real Claude Code registry or `ps` was read on a Linux machine.
  What exists: CI runs the fake-harness scenarios and the `PsTable` tests on ubuntu against a real process table
  (a symlinked `claude`, `lstart` in UTC), and they pass, so the ancestry, liveness and start-time logic works with
  real procps output. What was added here, by hand and on macOS, is `harness/linux_shaped.rs`: the same code against
  a made-up table shaped like procps (a padded `lstart` day, a bare `comm` cut to 15 characters, state letters such as
  `Ssl` and `I`, `HH:MM:SS` CPU times, a `linux` pid domain, an ssh tree under systemd). Those values come from
  knowledge of procps, not from a Linux run, and the file says so. A record from another pid domain reads as
  `OtherDomain`, never dead and never live, so a wrong guess about the Linux domain name fails safe (discovery finds
  no session). **Unobserved, ticketed as #797 (needs a Linux host):** the `pidDomain` a real Linux record carries; what
  `comm` shows for the harness (a native binary named `claude` passes, a script run by `node` would print `node`
  and read as not the harness); whether `procStart` equals `TZ=UTC ps -o lstart=` or comes from `/proc`; and the
  `ps -A` option set on procps and busybox.


## 18. First two-agent run (2026-11-13)

Method: two headless `claude -p` sessions (Claude Code 2.1.323, macOS) named alpha-run1 and beta-run1, started from the klatch repository with a throwaway `--settings` (hooks calling `klatch --discover hook`), `--setting-sources local`, `acceptEdits` and a narrow tool allowlist; scratch store and scratch repo in a temp directory; protocol in `experiments/two-agents/PROTOCOL.md`. The observer only read the room.

- **OBSERVED.** `--discover` resolved both headless sessions to distinct member names (`alpha-run1`, `beta-run1`) from the registry; both unattributed.
- **OBSERVED.** Six messages in 26 seconds: both proposed the same split independently, each claimed only its own file, released after editing, no conflict. The final result passed: 12 tests (independently re-run by the observer).
- **OBSERVED.** An agent that could not run pytest asked its peer in the room and the peer ran it. The room worked as the only channel.
- **OBSERVED.** Permission rules refused commands containing shell variables (`$KLATCH_BIN`, `$EXP`); both agents fell back to literal paths from the protocol (#174).
- **OBSERVED.** The klatch repository stayed clean; no key file or environment was printed.
- **NOT OBSERVED.** Whether the hook delivered anything (#175), claim collisions (#176), idle and resume (#177).

## 19. Run two: finding each other through the lobby (2026-11-13)

Method: protocol experiments/two-agents/RUN-TWO.md; three headless claude -p sessions (Claude Code 2.1.323, macOS), each started in its own scratch repository, throwaway settings with a hook wrapper that logs every call (time, session, event, exit, bytes, output), narrow tool allowlist, absolute paths and no shell variable in any command. The observer only read the rooms and files.

- **OBSERVED.** The hook fired for all three sessions on SessionStart, UserPromptSubmit and every PostToolUse (25 calls, exit 0 each). With no shell variable in the commands the agents used the absolute path as instructed, so #174 did not recur.
- **OBSERVED.** The hook delivered a peer's ask to two other sessions, with the data warning and sender, on the next tool call after it was posted (same second in the log). Neither answered, because the ask (read access to a directory) could not be answered by a peer.
- **OBSERVED.** All three followed the claim convention: claimed their files and the ticket before editing, released after, and posted a done message. No claim conflict occurred.
- **OBSERVED.** No agent needed to find another: alpha's dependency was written by beta in seconds, so nothing blocked it. The scenario was too easy and too fast to test discovery (#195).
- **OBSERVED.** gamma was blocked by headless read permission on a sibling repository (a run design flaw; comment on #178), asked in the lobby, got no reply and stopped.
- **OBSERVED.** No agent edited outside its own repository; no key file or environment was printed; no room of two was created.
- **OBSERVED.** The hook repeated unchanged claims on 13 of 25 calls with a changing countdown (#194).
- **NOT OBSERVED.** The asks agents write when really blocked, the herd, claim-the-ask, rooms of two.

## 20. Run three: a real blocking dependency (2026-11-13)

Method: `experiments/two-agents/RUN-THREE.md`; three headless `claude -p` sessions (Claude Code 2.1.323, macOS) in three scratch repositories, hook wrapper logging every call, absolute paths, no shell variable in any command. The first attempt failed at launch (a bad `--add-dir` argument in my script, so only beta ran and finished alone); it was discarded and the run redone, so everything below is the second attempt. The observer only read rooms and files; the agents were stopped with `kill -9` after about twelve minutes.

- **OBSERVED.** The block was real: alpha and gamma, told only that another agent owned the package they needed, each posted a `seeking:` ask within seconds (alpha wording: location and import name of the package that provides `slugify`; gamma: owner and API of the package it was documenting).
- **OBSERVED.** The asks reached beta through the hook within seconds (hook log, and a `hook_success` attachment in beta's transcript), but beta never answered. Its final report said nobody had posted anything. Whether the model saw the hook text is unknown (#199). The hook had advanced beta's cursor past both asks, so `read --peek` as beta later showed only the later ones: the asks were lost to `read` (#198).
- **OBSERVED.** Claims worked as a discovery signal: gamma named beta in a second ask (`@beta-run3b`, addressed to the owner) and alpha said beta looked like the owner because it had released claims on `text.py`. Both learned the owner from claims shown by the hook, not from a reply.
- **OBSERVED.** Neither blocked agent gave up: both re-asked and waited, still running after more than eleven minutes, and nobody told the human (#200).
- **OBSERVED.** After #194 the hook was quiet: 34 of 41 calls printed nothing, and no unchanged claim was repeated (run two: 13 of 25 calls repeated).
- **OBSERVED.** No room of two was created; no claim-the-ask occurred (no one answered); nobody edited outside their own repository; no key file or environment was printed; `slugify` and `wrap` were written and tested by beta (44 tests), while alpha and gamma produced nothing.
- **NOT OBSERVED.** The herd, claim-the-ask, rooms of two, and whether a busy owner is usefully interrupted, because the owner never saw the asks as asks.

## 21. Canary: does PostToolUse hook output reach the model? (2026-11-13, Claude Code 2.1.323)

Setup: throwaway `--settings` with `--setting-sources local`; hooks on SessionStart, UserPromptSubmit and PostToolUse each printed an instruction to include a distinct word in the final answer. One settings file printed plain stdout, the other printed JSON `{"hookSpecificOutput":{"hookEventName":...,"additionalContext":...}}`. Headless `claude -p`, task "run `echo hello`, then say done", two runs each.

- **OBSERVED (both runs).** Plain stdout: the SessionStart and UserPromptSubmit words reached the model; the PostToolUse word never did.
- **OBSERVED (both runs).** JSON `additionalContext`: all three words reached the model, including PostToolUse.
- **Consequence.** `klatch hook` prints plain text, so on PostToolUse its output is not seen by the model in headless sessions. This explains run one ("no message reached me") and run three (beta never acted on asks) without any model misbehaviour.
- **NOT OBSERVED.** Interactive sessions were not tested (cannot be driven headless); earlier notes saying delivery worked interactively remain unverified for PostToolUse.

## 22. Run four: hook delivery works end to end (2026-11-13)

Setup in `experiments/two-agents/RUN-FOUR.md`: four headless agents, alpha (api) and gamma (docs) blocked and not told where the dependency is, beta and delta both working in the `lib` repository. Binary built from the branch with #204 (`hook --event`) and #198 (separate hook cursor, PR #205).

- **OBSERVED.** The model saw hook output. alpha and gamma posted `seeking:` asks at the start; beta and delta received them through the hook within seconds (hook log, 06:46:06 and 06:46:07 local); beta claimed `ask:slugify-location` ten seconds later and posted both answers 52 seconds after the first ask (lobby seq 1 to seq 4). alpha and gamma used the answers: `cli.py` imports `slugify` from the path beta gave, all tests passed, and gamma's README matches the real API of `text.py` and `trunc.py` (delta's API came from an announcement beta pointed gamma to).
- **OBSERVED.** Every one of the four tickets was finished and tested (87 and 103 tests in `lib`), with no edits outside an agent's own repository (file hashes before and after) and beta and delta each kept to their own files in the shared directory.
- **OBSERVED.** Claims were used as intended: each agent claimed tickets and files, released them after, and the hook showed each claim to a peer once (a claim appeared at most twice in one session's log, where a renewal changes the expiry). 49 of 68 hook calls printed nothing.
- **OBSERVED.** A second capable agent stayed quiet. delta received both asks and did not answer; it reported that beta had already claimed both. No claim conflict happened.
- **OBSERVED.** Room of two (`dm-<a>-<b>`) was not used: answers were short enough for the lobby.
- **OBSERVED, a gap.** Sessions linger after finishing. The agents ran a loop of six `read` plus `sleep 30` in one background shell command to wait; the model cannot cancel it when the answer arrives, so alpha and gamma sat for minutes after their work and last post (gamma's session ended about three minutes after its README). The instruction I gave ("wait with sleep 30, at most 6 times") invited this.
- **NOT OBSERVED.** The herd and claim-the-ask under contention: delta's quiet could be deference to the existing claim (beta claimed at 06:46:17 and delta saw the claim at 06:46:19) or just that it was busy, and a claim conflict did not occur. Interactive sessions. Whether an unclaimed ask would be answered twice.
- **Caveats.** gamma's README example outputs were traced by hand and not run (its own report says so). The agents knew the lobby convention from the task text; this does not show they would follow it unprompted.

## 23. Run five: claim-the-ask under contention (2026-11-13)

Setup: two idle helpers (rho and sigma, same read-only `lib` package with `text.py`) waiting in `klatch wait lobby`, then two askers (alpha in `api`, gamma in `docs`) released together once both helpers had joined. Convention change from run four: the claim is `ask:<N>` where N is the ask's sequence number, so two helpers claiming the same ask always overlap. Binary from the branch with `klatch wait` (PR #211). Headless, throwaway `--settings`, hook log on every call.

- **OBSERVED.** Both asks (seq 1 and 2, posted in the same second) woke both helpers. One helper (`lib-c3`) claimed `ask:1` and `ask:2` and posted both answers within 10 seconds of the asks (seq 3 and 4). The other helper (`lib-49`) tried to claim both, was refused with a conflict each time, and stayed silent. One answer per ask; no duplicate answers. The claim, not politeness, decided it: the loser's own report says its claims were refused.
- **OBSERVED.** The loser then checked the two answers against `text.py` and found them correct, and stopped after four more waits (4 x 90 s) with nothing new.
- **OBSERVED.** The same helper won both asks: it was first both times. Whether the other would win if it were first, and what happens when two claims land in the same instant, is not shown by one run (the store serialises claims, so one must win; the run does not test the instant).
- **OBSERVED.** `klatch wait` fixed the lingering sessions of run four: alpha ended after 28 s and gamma after 30 s, both right after the answer. The helpers lingered only because their task told them to keep waiting.
- **OBSERVED.** alpha's tests pass (5), gamma's README describes the real module, `lib/text.py` was not modified (hash), and nobody edited outside their own repository.
- **NOT OBSERVED.** A tie within the same poll (200 ms); more than two helpers; a helper that is busy when the ask arrives; an ask no one can answer; whether agents would use `ask:<N>` without being told (#209). The `ask:<short name>` convention of runs three and four can miss a conflict when two helpers pick different names; the sequence number removes that, and is worth making the documented convention.

## 24. Run six: wait v2 in the herd (2026-11-13)

Setup: two idle helpers on the same `lib` package and two askers (alpha in `api`, gamma in `docs`) released together, as run five, on the binary with `klatch wait` v2 (PR #218). Helpers waited with `wait lobby --only seeking: --timeout 60`. alpha needed a `slugify` that transliterates accents (the existing one dropped them), so the owner had to change the package. Headless, throwaway `--settings`, hook log on every call.

- **OBSERVED.** The filtered wait woke both helpers for the two asks. One helper (`lib-a1`) claimed `ask:1` and `ask:2` and posted both answers 16 to 18 seconds after the asks (seq 3 and 4); the other helper's claims were refused on both and it stayed out. The same pattern as run five: one winner, one refused loser, no duplicate answers.
- **OBSERVED.** The winner changed the package to meet alpha's need: `text.py` now transliterates accents with NFKD, with tests; `uvx pytest -q` passes in `lib` (4) and `api` (4), and `cli.py "Café Crème"` prints `cafe-creme`. gamma's README mentions the accent handling, so it was written from the updated module.
- **OBSERVED.** A room of two was created and used: the helper moved to `dm-api-30-lib-a1`, told alpha the import path and what changed; alpha confirmed in the same room.
- **OBSERVED, a gap.** One helper waited on `lobby dm-api-30-lib-a1 --only seeking:`. The filter applies to every named room, so a reply in the room of two that does not start with `seeking:` would not have woken it. This run did not need it, but it is a trap (see the ticket).
- **OBSERVED.** The helpers lingered (240 s and 289 s) only because my stop rule told them to wait for three timeouts; the askers ended in 48 s and 54 s.
- **NOT OBSERVED.** A multi-step exchange in the room of two: alpha's ask already said accents should be transliterated, so the helper had nothing to ask back. The scenario did not force a question (#210 stays open). Two helpers racing within one poll; a third helper; whether agents would use any of this unprompted (#209). Update (2026-11-24): a multi-step exchange in a room of two was run on 2026-11-21 (#210 variants A to C; an explicit instruction produced the room of two with 4 question and answer pairs, no instruction kept the exchange in the lobby); see the research note for #210.

## 25. Run seven: how a helper knows it is finished (2026-11-13)

Question (#221): which completion signal ends an idle helper cheapest? Six stop rules, each with two real helper agents and two scripted askers (the same two asks), binary with `wait` v2. Per variant the helpers had the same task text except the stop rule; at most 12 `wait` calls each. Askers were scripts so the only variable is the helper's rule; the signal itself (closing message, `status: done`, `closed #N`) was posted by my script after both asks were answered.

| Variant | Stop rule | Turns (both helpers) | Cost, both | Sum of helper time | Longest exit after the signal |
|---|---|---|---|---|---|
| V0 | 3 timeouts in a row (baseline) | 26 | $0.325 | 452 s | 197 s |
| V1 | a `closing:` message | 17 | $0.268 | 75 s | 3 s |
| V2 | presence: askers last seen over 60 s ago | 26 | $0.340 | 222 s | 73 s |
| V3 | asker posts `status: done` (member status) | 21 | $0.273 | 58 s | 5 s |
| V4 | asker posts `closed #N` (ask state) | 21 | $0.291 | 68 s | 6 s |
| V5 | told to expect exactly 2 asks | 20 | $0.261 | 54 s | 1 s |

- **OBSERVED.** Every rule with an explicit signal (V1, V3, V4, V5) ended both helpers within about 1 to 6 seconds of it. The timeout rule (V0) left helpers waiting 190 to 200 seconds after the answers, and presence (V2) about 70 seconds, because it must wait out the quiet period.
- **OBSERVED.** In every variant the claim worked: no ask was answered twice; in V2, V4 and V5 the two helpers split the asks, one answering each, and in V0, V1 and V3 one helper answered both.
- **OBSERVED.** Cost differences are small (all between $0.26 and $0.34 for two helpers), because a `wait` call is one cheap turn. The saving is wall time and idle sessions, not tokens. Turn counts ranged from 17 (V1) to 26 (V0 and V2).
- **Reading, with caveats.** One run per variant, so the differences among V1, V3, V4 and V5 (17 to 21 turns, $0.26 to $0.29) are within what model variance could produce; only the gap between those four and V0 and V2 is clear. V1 and V5 look cheapest because they ask the helper to check nothing: V1's signal came free from a script (in a real team someone must post it), and V5 requires knowing how many asks will come. V3 and V4 let the asker say it is done, so no outside party is needed, at the cost of the helper reading the lobby to check; they cost about 4 more turns than V1 in this run.
- **NOT OBSERVED.** V3 and V4 with real askers (the scripts posted the done and closed messages, so whether an agent asker remembers to post them is untested); more than two asks; a helper whose asker never sends the signal (the 12-wait cap was not reached in any run).

### 25.1 The combination V1 + V5 (same day)

Rule: the helper is told to expect exactly 2 asks and stops when 2 have an answer or as soon as a `closing:` message appears, whichever is first. Two real helpers per case, scripted askers.

| Case | What arrives | Result |
|---|---|---|
| c-a | 2 asks | Both asks answered (one by each helper); helpers ended 1 second after the second answer, on the count. 17 turns, $0.25 |
| c-b | 1 ask (fewer than expected) | The count never reached 2; both helpers ended 3 seconds after the script posted `closing:`. 16 turns, $0.24 |
| c-c | 3 asks (more than expected) | Both helpers stopped after answering 2 asks; ask #3 ("which module has `wrap`") was never answered and nothing said so. 15 turns, $0.26 |

- **OBSERVED.** The combination works when the count is right (c-a) or too high (c-b, rescued only by the closing message). It silently drops work when the count is too low (c-c): the helpers had done what they were told, and no one learned ask 3 was unanswered.
- **Reading.** A count is only as good as whoever supplies it, and `closing:` needs a human or a script that knows the work is over, so for the aim of minimal human intervention the pair puts a human in the loop exactly in the case c-b (too few asks). A rule that looks at the asks themselves (V3 member status, V4 ask state) has neither problem, because it does not need to know the number of asks in advance.
- **NOT OBSERVED.** V3 or V4 with real asker agents; an ask that arrives after the helpers have decided to stop, under any rule.

### 25.2 V3 and V4 with real asker agents (same day)

Two real helpers and two real asker agents (alpha: `cli.py`; gamma: README) per variant, no scripts. The askers were told in their task text to post the signal themselves: V3 `status: done` when completely finished, V4 `closed #N` (their own ask number) once the answer was used. Helpers had the V3 or V4 stop rule from run seven, at most 12 `wait` calls.

| Variant | Helper turns | Helper cost | Asker turns | Asker cost | Helpers ended, relative to last asker exit | Work |
|---|---|---|---|---|---|---|
| V3 `status: done` | 12 + 10 | $0.301 | 9 + 9 | $0.298 | -1 s and -1 s | `uvx pytest` passes (4), README written |
| V4 `closed #N` | 13 + 12 | $0.324 | 10 + 11 | $0.320 | +1 s and -2 s | `uvx pytest` passes (3), README written |

- **OBSERVED.** Both askers posted the signal themselves, in both variants (lobby seq 5 and 6 in each), and the helpers stopped as the last asker finished: within about two seconds either side of the asker's exit. No helper reached the 12-wait cap, no scripted signal was needed, and no human was involved.
- **OBSERVED.** The claim worked as before: V3's one helper answered both asks, V4's helpers split them. No duplicate answers.
- **OBSERVED.** The two variants cost about the same ($0.60 and $0.64 for the four agents); the 1 to 3 extra turns in V4 are within what noise could give. Neither is clearly cheaper.
- **Caveat.** The askers were told to post the signal; this shows agents carry it out when told, not that they would invent it (#209). The signal convention has to reach every asker.
- **NOT OBSERVED.** An asker that crashes or forgets (the 12-wait cap then ends the helper, at the cost of the full wait); an ask that arrives after the helpers stopped; more than two askers.

### 25.3 A late ask after the helpers have stopped (same day)

Under the V3 rule: two helpers and one asker run normally; the helpers post `status: done` (saying they are leaving) and exit; five seconds later a second asker starts. Two late-ask cases, because the first pair (`l0`, `l1`) used a question the lobby history had already answered (the late asker read the earlier answer in the history and wrote its README without a new answer, so it did not test an unanswered ask). The second pair (`m0`, `m1`) asks for something only a helper could do (add `reverse_words(text)` to the package).

| Late asker | What it did | Turns | Cost | Time |
|---|---|---|---|---|
| m0: plain convention | Posted the ask, waited 4 x 60 s, nobody answered, reported to the human that it could not proceed | 8 | $0.148 | 217 s |
| m1: told to read the lobby history and check `who` first | Saw both helpers had posted `status: done`, did not post or wait, reported at once that no helper is available | 3 | $0.092 | 16 s |

- **OBSERVED.** With the V3 rule a late ask is lost: nobody is there. The plain asker found out only after four full waits, then told the human, so the human was the escalation path (against the minimal-intervention aim, though only after a real failure).
- **OBSERVED.** A late asker that checks first learns in seconds. It used the helpers' own `status: done` messages in the lobby history; its one `who` call failed (it omitted the room name) and it did not need it. The check cost 3 turns and $0.09 against 8 turns and $0.15 for waiting it out.
- **OBSERVED.** The lobby's durable history can itself answer a late ask (`l0`): a question already answered once costs nothing the second time.
- **Caveat.** One run each; the checking asker was told to check, so this shows the idea works, not that agents would do it unprompted (#209).
- **NOT OBSERVED.** Anything that re-starts a stopped helper when a late ask arrives (needs the harness or a supervisor, not klatch alone).

## 26. Run nine: a declined ask stays open (2026-11-13)

Tests the decision to drop "a released `ask:<N>` claim ends the ask" (DESIGN section 25.4). Three real agents on the binary with open asks (`klatch asks`, the stale-ask report, the session-start backlog): an asker (alpha), a helper told to claim the ask and then decline it (release the claim, post `declined #1: not my area`, post `status: done`, stop), and a second helper started 65 seconds later.

- **OBSERVED.** After the decline, `klatch asks lobby` still listed the ask, unclaimed, 11 seconds after it was posted and again 76 seconds after (the helper had claimed, released and left). A released claim did not end it.
- **OBSERVED.** The asker was told by klatch: its next hook output (PostToolUse, JSON context) said "your ask #1 has had no claim for 76s: seeking: ...". alpha quoted it exactly in its final report, so the stale-ask line reached the model. Reported once: the asker's later hook lines carried only claim changes.
- **OBSERVED.** The late helper answered the ask (`answer to #1:` with the location, module and API), claimed and released `ask:1`, posted `status: done`, and stopped; alpha used the answer, `uvx pytest` passes (5), and alpha closed the ask (`closed #1`). `klatch asks` then listed nothing.
- **OBSERVED, a gap.** The late helper's SessionStart hook printed nothing (0 bytes): the backlog works only for rooms the member has already joined, and a brand-new session has joined none. It found the ask only because its task told it to run `klatch asks lobby` as a fallback. The backlog as built does not serve a new helper (ticket).
- **OBSERVED, a bug.** A `wait` with `--only` or `--peek` does not refresh the waiter's presence in `who` (it does not touch it), while an unfiltered `wait` does; a helper waiting with a filter looks absent. Checked by hand after the run: `last seen 5s ago` after a 2 s filtered wait, `0s ago` after an unfiltered one (ticket).
- **NOT OBSERVED.** More than one open ask; a stale ask when the asker has several rooms; the ask text growing past 160 characters in a real run (covered by a unit test).

## 27. The board in a real session (2026-11-13)

Two headless asker runs, one scripted room with two offers (`text-helper`: slugify and wrap; `other-helper`: image resizing). Task: join, run `klatch board lobby`, name the member whose current offer fits a slugify need and who is not done.

- **OBSERVED.** With both offers current, the agent named `text-helper` (3 turns, $0.075).
- **OBSERVED.** After `text-helper` posted `status: done`, the agent said NO HELPER AVAILABLE and explained why (3 turns, $0.077). One call to `board` gave what run eight's late asker needed several turns of lobby history to find.
- **NOT OBSERVED.** A requester addressing the helper in a room of two after the board (it only named them); an offer going stale in a real run (#250).

## 28. A new helper sees the session-start board (2026-11-13)

A scripted asker posted an ask and an unrelated offer; then a real headless helper with no joined room started with the hook `klatch hook --event SessionStart --board lobby` (and told not to run other klatch commands before joining).

- **OBSERVED.** The helper's SessionStart context carried the board (437 bytes: the data warning, the open unclaimed ask, the offer). It joined, claimed `ask:1`, answered, released, posted `status: done`, in 8 turns for $0.129, without being told to run `klatch asks` (run nine's late helper needed that fallback).
- **OBSERVED.** Before it joined, the hook read the board without making it a member.
- **NOT OBSERVED.** A session with `--board` and no open ask (the board shows nothing to do); the aging in a real run (unit and gateway tests cover it); `init` writing the flag (#65).

## 29. Declines: busy versus nobody capable (2026-11-13)

Two headless asker runs; the helpers' decline messages were posted by a script (the asker is the real agent). Task: post an ask, wait up to 6 times; if the hook says nobody here may be able to do this, stop and tell the human; if a helper says it is busy, keep waiting.

- **OBSERVED (zero capable).** Two helpers posted `declined #1 cannot: ...`. After the second, the asker's hook said "your ask #1: nobody here may be able to do this (2 helpers declined as not able): ...", and the agent quoted it exactly, stopped after 2 of 6 waits (27 s, 5 turns, $0.10) and told the human it must look elsewhere.
- **OBSERVED (a capable helper is busy).** One helper posted `declined #1 busy: ...`, another `declined #1 cannot: ...`; no escalation line appeared, the asker kept waiting, and a third helper's answer arrived and was used (66 s, 6 turns, $0.108).
- **NOT OBSERVED.** Real helper agents choosing the class word unprompted (they were scripted, and #209 covers whether agents follow conventions unprompted); a `failed` outcome in a run; whether helpers classify a decline correctly (busy versus cannot) when judging for themselves.

## 30. First interactive trial of `klatch init` (2026-11-13, Claude Code 2.1.323)

`klatch init --scope project --yes` in the klatch repo, binary copied to `~/.local/share/klatch/klatch`. Two new interactive sessions, then `klatch hooks --of <member>` (#266).

- **OBSERVED.** The hooks fire automatically with no instruction and no join: both new sessions recorded `UserPromptSubmit` then `PostToolUse` on their first prompt, and printed 0 bytes (nothing to deliver, since their only unread message had been read). Before the record existed, nothing could show this: the hooks print nothing until a session has joined a room, Claude Code's transcript keeps a hook record only when there is output, and a process monitor polling every few milliseconds caught nothing in 10 minutes.
- **OBSERVED.** After a session joined the lobby, a message from another member reached it on its next tool call and the model quoted it as data (the first interactive proof for #199).
- **OBSERVED, correcting an earlier claim.** The session in which `init` was run (started before the settings file existed) also recorded `UserPromptSubmit` and `PostToolUse` afterwards, so Claude Code applies a changed project settings file to a session that is already running. The statement that only new sessions would be affected was wrong.
- **NOT OBSERVED.** `SessionStart`: the sessions had started before the recording existed. A session started after the swap is needed. Joining without being told (#267), the board at session start in an interactive session, an idle session being woken (#214).

## 31. The convention taught only by klatch (2026-11-13)

Runs twelve and thirteen: a helper and an asker whose task text says nothing about klatch commands or the convention ("use whatever klatch gives you"). Everything else came from the SessionStart hook: the board and the convention text (DESIGN section 25.11). Same scenario both runs; run thirteen added the exact commands to the text.

| | Helper turns, cost | Asker turns, cost | Wasted turns |
|---|---|---|---|
| Run twelve (actions named, commands not) | 13, $0.193 | 14, $0.182 | `--help` on both sides; the helper tried `claim ask:2` three times before it found the room argument |
| Run thirteen (exact commands) | 11, $0.158 | 9, $0.161 | none: no `--help`, every command right first time |

- **OBSERVED (both runs).** Nothing in the task text taught the convention, yet: the helper read its repository, then posted one concrete `offering:` (not a generic one, and not at start); the asker posted `seeking:` and waited with `wait`, never `sleep`; the helper claimed `ask:2`, answered with `answer to #2:` and released; the asker posted `closed #2` and `status: done`; the helper withdrew its offer and left with `status: done`. The asker's tests pass (6 and 5).
- **OBSERVED.** The only defect was in my text, not in the agents: it named the actions but not the command forms, so run twelve burned turns guessing. Showing the exact commands removed the wasted turns (asker 14 to 9 turns).
- **Caveats.** One run each, so the 14 to 9 and 13 to 11 turn drops could include model variance; the removal of `--help` and the wrong claim form is a direct effect of the text. The helper's task text did say to stay available until the askers were finished (the stop rule is not part of the convention; it follows the V3 `status: done` signal). Two agents only: no contention, no declines.
- **NOT OBSERVED.** More than one helper, declines, the dual role (#246), a session that ignores the convention, longer lobbies. Update (2026-11-24): the dual role was run on 2026-11-21 for two and three members (#246); see the research note for #246.

## 32. A deadlock trap: opposite-order claims (2026-11-13)

Run fourteen (#246). Two real agents, one shared directory with `X.py` and `Y.py`, convention taught only by the SessionStart hook, task text only saying to claim a file before editing it. Alpha must edit X then Y and keep its claim on X until Y is done; beta must edit Y then X and keep its claim on Y until X is done: hold-and-wait in opposite order.

- **OBSERVED.** The deadlock formed for real: each claimed its first file and edited it, then each got a claim conflict on the other's file (the conflict message names the holder and the seconds left).
- **OBSERVED.** The agents resolved it by themselves in the lobby in about a minute (alpha 69 s, 18 turns, $0.237; beta 51 s, 15 turns, $0.202). Both posted the deadlock; the proposals crossed ("our messages crossed"); they agreed that the one who had already edited its first file goes first; the other reverted its edit and released; the first finished both and released; the second then did its own. No human involved, no claim lapse needed.
- **OBSERVED.** The final files say `from alpha` in both: the second agent's later edits replaced the first agent's. Nothing in klatch (or in my task text, which required both changes "to land together") made the first agent's result survive; the claims prevented concurrent edits, not a sequence of overwrites.
- **OBSERVED.** Crossed proposals: each offered to yield to the other at the same moment. A deterministic tie-break was not available to them and cost a message round.
- **NOT OBSERVED.** Three or more members in a cycle; a cycle through asks (A waits on B's answer, B waits on A's) rather than through claims; a member that is busy for minutes and cannot answer; agents that do not negotiate (a crashed holder, a claim that outlives its owner by the 600 s TTL). Update (2026-11-24): a cycle through asks (#273) and a two-member and a three-member ask cycle (#246) were run on 2026-11-21 with real agents; a crashed holder and a resolved three-member cycle are still not observed; see the research note for #273 and for #246.

### 32.1 The same trap with deadlock detection (run fifteen)

Same scenario, binary with detection (DESIGN section 25.12) and the convention line about conflicts.

| | Turns | Cost | Time | Lobby messages about the deadlock |
|---|---|---|---|---|
| Run fourteen (no detection) | 33 (18 and 15) | $0.44 | 69 s and 51 s | 6, with crossed offers to yield |
| Run fifteen (detection) | 25 (18 and 7) | $0.34 | 108 s and 91 s | 0 |

- **OBSERVED.** The refusal that closed the cycle carried the deadlock text, and that agent (alpha) did what it said: restored `X.py` to `initial`, released, and waited; the other agent (beta) finished both files and released. No negotiation was needed and nothing was posted about the deadlock.
- **OBSERVED.** Fewer turns and less cost, but longer wall time (beta held its first claim and waited about a minute on purpose, because its task said the two changes must land together; it did not follow the convention line about releasing before waiting). One run each: the turn and cost drop could include variance; the absence of negotiation messages is a direct effect.
- **OBSERVED, still true.** Both files end up `from alpha`: the second agent's edits replaced the first's. Detection resolves the wait, not the semantic conflict.
- **NOT OBSERVED.** Three or more members; a cycle through asks; a crashed holder. Update (2026-11-24): a cycle through asks (#273) and a two-member and a three-member ask cycle (#246) were run on 2026-11-21 with real agents; a crashed holder and a resolved three-member cycle are still not observed; see the research note for #273 and for #246.

## 33. The SessionEnd hook in a real session (2026-11-13, Claude Code 2.1.323)

A scratch project with `klatch init --scope project --yes`, then one headless Claude Code session in it ("reply ok") that ended normally.

- **OBSERVED.** The session joined the lobby at start by itself (SessionStart) and, after it ended, `who lobby` showed it as `status: done session ended`, written by the SessionEnd hook using the session's discovered identity (so the registry record is still readable when the hook runs).
- **NOT OBSERVED.** An interactive session closed with `/exit` or Ctrl-D; a crash or `kill -9` (no hook runs, the aging backstop applies); a session that ends while it holds claims (the hook does not release claims; they lapse by their TTL, #59).

## 34. Headless sessions, and a bug the first test hid (2026-11-13, Claude Code 2.1.323)

- **OBSERVED.** All four hooks (SessionStart, UserPromptSubmit, PostToolUse, SessionEnd) fire in headless `claude -p` runs, with project settings, with `--settings`, with `--setting-sources local`, and with the user's default sources and plugins loaded. A headless session joins the lobby at start and (after the fix below) is marked done at exit.
- **OBSERVED, a bug (#284).** A headless child started from an interactive session marked its parent done. The first SessionEnd test (a scratch project, only klatch's hooks) passed three times out of three because the hook won a race against the harness removing its registry record; with the user's real settings and plugins the record was gone first, the walk reached the parent's record, and the parent was marked done (and the child was not).
- **OBSERVED, fixed.** With the session id from standard input and a remembered session map, the same headless child run (real settings, started from this interactive session) recorded all four events for the child, marked the child done, and left the parent active.
- **Lesson.** The test that passed was too kind: it had no ancestor session to fall through to. The bench needs a fake harness with a nested parent and child (ticket).

## 35. A real-agent run of leases and clarification (2026-11-13, Claude Code 2.1.323)

One helper (owns a package with two slug variants) and one asker (needs the ASCII variant, told to say so only if asked). Both finished, 7 and 9 turns. The helper volunteered both variants in its offer, so the asker never needed `need-info`: the clarification words were not exercised by a real agent in this run. The ask, answer, closed and status path worked. Clarification and lapse behaviour are covered by the bench scenarios (scripted); a real-agent run that forces `need-info` stays open on #235.

## 36. What Claude Code exposes about a running session (2026-11-13, Claude Code 2.1.323)

Hooks: no timer, heartbeat or per-tool-call tick; one idle notification. Registry record: `status` busy or idle with the time of the last change (watched for 90 s while busy: unchanged, so not a heartbeat). `claude agents --json` lists interactive and background sessions with the same status, and `blocked` states for background ones; seven sessions on this machine, two forgotten background ones. Process view: `ps` state, CPU time and child processes are readable; the transcript file grows as a session works. No progress inside a turn is reported by Claude Code. See DESIGN section 29.

## 37. Resume and fork identity, in Claude Code and Codex (2026-11-13, Claude Code 2.1.323, Codex 0.147.0)

`experiments/identity/resume_fork.sh` measures it. Claude Code: `--resume <id>` keeps the session id, `--resume <id> --fork-session` gives a new one. Codex: `codex exec resume <id>` reports the same thread id (it does so before it needs an API key, so the check works without a login); `codex fork` is interactive only and was not run. So misrouting by a reused id does not arise: a resumed session is the same klatch member, a fork is a new member (bench scenarios `a_resumed_session_keeps_its_member_and_its_cursor_with_a_new_process` and `same_name_distinct`).

What Codex offers a future klatch adapter (read from the binary and `~/.codex/hooks.json`, not exercised): hooks for SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, Stop, SessionEnd, PreCompact and subagent events; each hook's standard input carries `session_id`, `transcript_path`, `cwd`, `hook_event_name`, `permission_mode` and, at start, a `source` of startup, resume, clear or compact; shell commands run by Codex see `CODEX_THREAD_ID`. There is no registry of live sessions keyed by pid like Claude Code's, so the ancestry walk (RESEARCH-NOTES section 17) does not apply: identity would come from the hook's session id, or from `CODEX_THREAD_ID` for a command the agent runs.

Found by the new scenario: the SessionEnd hook used the digest to learn the member's rooms, which marks unread messages as shown, so a resumed session never saw a message that arrived just before the old one ended. It now lists the rooms without reading.

## 38. Peer identity from a gateway-owned socket (2026-11-13, macOS 14, experiment 14.3, ticket #29)

`klatch-harness/tests/peer_identity.rs` shows it works. A gateway that owns a Unix socket can ask the kernel for the pid of the process at the other end of an accepted connection (`getsockopt` with `LOCAL_PEERPID` on macOS, `SO_PEERCRED` on Linux; the Linux branch ran in CI on ubuntu and passed). That pid is the connecting process, here the script a session's shell started, not the session itself; the existing ancestry walk (`find_session`, with the same liveness checks) then maps it to the session's registry record, giving the session id, name and directory. A process with no session above it maps to nothing and stays unattributed.

What this gives and does not give: the identity is as strong as the process tree and the registry, which are same-user evidence (a process can lie about its ancestry only by being the user, who can do anything anyway). It is not proof against another process of the same user that arranges to descend from a session. It needs a gateway that owns a socket, which klatch does not have yet (the CLI opens the store directly); it is the identity source for that daemon, when it exists (epic #152), where the socket's file mode also admits only its owner.


## 39. A live run of the wake watcher and the Channels pages (2026-11-14, Claude Code 2.1.325, ticket #37, #38)

- **OBSERVED.** `claude -p --input-format stream-json --output-format stream-json --verbose --model haiku` with a `SessionStart` hook running `klatch wake --timeout 120` as `async` and `asyncRewake` (timeout 150), a private `KLATCH_HOME`, and the member found by `--discover`. After the first turn ended, another member joined the lobby and posted. 1.7 seconds later the session emitted a `hook_response` whose stderr was the klatch pointer (`[klatch] unread messages in room lobby; run klatch read lobby ...`), began a new turn and ran `klatch read lobby`. The post's text (a marker string) did not appear anywhere in the context before that read. This repeats the 14.1 result with a watcher that waits for a real condition instead of a sleep.
- **NOT TESTED.** The interactive terminal; a watcher kept alive past its hook timeout (it exits at 540 seconds by design); more than one idle session at once.
- **DOCUMENTED (read 2026-11-14).** Channels: research preview; claude.ai or Console authentication; unavailable on Bedrock, Vertex and Foundry; Team and Enterprise must enable; allowlist is `claude-plugins-official`; a custom channel needs `--dangerously-load-development-channels` with a confirmation dialog in an interactive session, and the flag is ignored under `-p` and the Agent SDK; notifications are not acknowledged and are dropped silently if the channel is not loaded or policy blocks it; an ungated channel is a prompt-injection vector, so the channel itself must check the sender.
- **OBSERVED (2026-11-14, re-arming).** A watcher started by a `Stop` hook (`async`, `asyncRewake`) woke the session as a `SessionStart` one does. Without a memory of what it had pointed at, the model did not read (headless permission denied), so every new watcher woke at once and the session looped, a turn a second, until killed. With the wait position recorded before the wake and a lock that keeps only the newest watcher per member: one wake, one turn, quiet.
- **OBSERVED (2026-11-14, invite).** A headless session with the wake hooks (SessionStart and Stop) and Bash for klatch allowed. Another member ran `klatch invite <session> design` in the lobby. The session woke, ran `klatch read lobby`, reported "an unverified invite from alice to join a room called design", said it was not an instruction from its user, did not join, and asked its user. `who design` stayed empty. So the invite reached a running session and did not act for it.
- **OBSERVED (2026-11-14, interactive terminal, Claude Code 2.1.325, Haiku 5.5, driven through tmux, private store, `init --wake`).** An idle session was posted to as another member. A new turn started by itself within 10 seconds of the post (poll every 2 seconds on the test side plus the watcher's 3 second look) and the model reported the message. A second and a third post each woke it again, once per new message, with no loop. Two things to know: the model took the watcher's stderr for "stop hook feedback" that asks it to run `klatch read lobby`, and the auto-mode classifier denied that command as "Unsanctioned Communication", so the session kept saying it was blocked on the read, while still reporting the message text that the prompt hook had shown. And a session started from a shell that inherits another Claude Code session's environment variables could not identify itself (`no member name given`); a clean environment (`env -i` with HOME, PATH and TERM) joined as expected.
- **NOT TESTED interactively.** A wake while a tool call runs (auto mode blocked every long command tried), two idle sessions at once, and an idle wait past 540 seconds (#357).
- **OBSERVED (2026-11-14, long idle).** Headless, `klatch wake --timeout 780` as an `asyncRewake` hook with hook timeout 900. After the first turn the session sat idle for 660 seconds, longer than the 600 second default; a post then woke it in 2.5 seconds. So a hook-started watcher outlives 600 seconds when its hook timeout is raised. Not measured: the full 3600 seconds.
- **OBSERVED (2026-11-14, two sessions and a background task, interactive, tmux, manual permission mode).** Two idle interactive sessions on one store both woke on one post. A session whose model had started a 40 second sleep (Claude Code ran it as a background shell, so the turn had ended) woke too, noted the message and left it alone. A foreground tool call that runs while a post arrives was not reached: the model backgrounds long sleeps. With the reworded pointer neither session said it was blocked on a read.
- **OBSERVED (2026-11-14, a post during a foreground tool call, interactive, tmux, manual permission mode, Haiku 5.5).** The model ran `python3 -c 'import time; time.sleep(45)'` in the foreground (`Running..., timeout 2m`). A post 12 seconds in did not interrupt it: the tool ran its full 45 seconds, the watcher's pointer then appeared as "Stop hook feedback" once the tool had returned, and the model's closing reply reported the message ("a hook also reported a message from bob") without a second turn. No loop. This closes the last untested wake case from section 39 apart from a full hour idle (#357 measured 11 minutes).

## 40. A live run of a credentialed worker (2026-11-14, Claude Code 2.1.325, ticket #121)

- **OBSERVED.** A launcher ran `klatch worker issue w-live --clearance internal --ttl 600 --parent lead --out <file>` (mode 600). A headless `claude -p` (Haiku) was started with `KLATCH_HOME` and `KLATCH_IDENTITY_FILE=<file>` and no name, no key directory and no harness registry lookup, and asked to run one `klatch post` through its Bash tool. The command posted; another member's `read` showed `[1 <ts> w-live] hello from the credentialed worker`, with no "(unverified)". `klatch worker list` showed the credential live with its clearance and parent. The worker read its store key and identity only from the bundle.
- **Side note.** The run printed a notice from Claude Code that this sign-in ends between 2026-11-20 and 2026-11-23, and that `claude auth login` or `claude setup-token` (for unattended jobs) should be used before then.

## 41. Proof-grade assurance for Rust, and where Hegel fits (2026-11-14, tickets #388, #401)

Researched on the day from each tool's repository, crates.io and docs; a note marks what could not be verified.

- **Hegel** (hegel.dev, github.com/hegeldev/hegel): a property-based testing engine, in beta (its README says breaking changes may come). The Rust crate is **`hegeltest`** 0.52.6, released 2026-11-13, MIT (the crate named `hegel` on crates.io is unrelated). It runs N generated cases (default 100), shrinks a failure and replays it from a test database. Passing is **randomized evidence, not a proof**; no exhaustive or symbolic mode was found on the pages read (the whole docs site and API reference were not read). On Intel macOS the engine builds from source.
- **Bend** (bend-lang.com, HigherOrderCO/Bend v2.0.40, Apache-2.0): its own language with dependent types, used as a proof checker for Bend programs. It checks Bend, not Rust, and has no Rust verification path. What was probably meant is a checker stronger than testing, which for Rust is the list below.
- **Proof or bounded proof for Rust:** Verus (SMT, MIT, active, needs annotations and its own toolchain), Creusot (Why3, LGPL-2.1, v0.14.0 2026-10-12), Kani (bounded model checking with CBMC, Apache-2.0, 0.68.0 2026-09-16, concurrency out of scope), Flux (refinement types, MIT, active), Aeneas (Rust to Lean or Coq, safe Rust only, very high effort). Prusti's last tagged release is 2023 and MIRAI is archived: skip both.
- **Model checking of a design:** Quint over Apalache (Apache-2.0, active), TLA+ with TLC (MIT), P, Alloy, Stateright (Rust, quiet since 2025-07). These check a model, not the code.
- **Concurrency and harnesses:** Shuttle and Loom for std threads; bolero runs one harness as a fuzz target, a property test and a Kani proof (its Kani support was not verified); proptest and cargo-fuzz for volume; cargo-mutants measures whether tests would catch changes.
- **For klatch:** provable or boundedly provable: path cleaning (idempotent, never escapes the root), the listener policy, label-chain dominance (an order), claim overlap (symmetric), the recovery-code and sealed-value round trips with the crypto calls opaque, and the decoder's "never panics" within a bound. Best use of a model checker: the claim and lease protocol (overlap, expiry, deadlock), as a model. Not provable end to end: SQLite, SQLCipher and OpenSSL are C, opaque to Rust verifiers, so the store is checked against a model with property tests.
- **Layers, as recommended:** (1) Kani under bolero harnesses for the pure functions, in a separate CI job, escalating only the critical few to Verus if unbounded proof is wanted; (2) a Quint model of the claim and lease protocol checked with Apalache, invariants named; (3) fuzz and Hegel or proptest for the decoder, with a Kani never-panics harness; (4) Shuttle or Loom for the thread parts; (5) Hegel with model-based tests against an in-memory model for the store, and cargo-mutants nightly to check the tests; (6) leave Aeneas, Prusti, MIRAI and any end-to-end proof through SQLCipher alone. Each layer states what it does not cover, and nothing property tested is called proven.

## 42. Hegel measured on klatch code (2026-11-14, tickets #388 to #391)

Section 41 read about Hegel; this one ran it. The scratch crate `experiments/hegel-bench` (outside the workspace, with
its own lockfile) depends on `hegeltest =0.52.6` with the `static-engine` feature and on klatch with only the `client`
feature, so its properties test the real `klatch::domain` and `klatch::protocol` code. macOS arm64 only; the Linux
build was not run.

**Build and supply chain.**
- The default build of `hegeltest` has a `build.rs` that runs a nested `cargo build` of the engine crate `hegeltest-c`
  at build time, resolving that crate's own dependencies from crates.io **outside our `Cargo.lock`**, so neither
  `--locked` nor `cargo-deny` sees them and the build needs the network. This run's nested lockfile happened to match
  ours exactly (no divergence observed), but nothing guarantees that later.
- The `static-engine` feature links the engine as an ordinary dependency instead. Then all of it is in our lockfile:
  **26 crates beyond klatch's own** (hegeltest, hegeltest-c, hegeltest-macros, dashu-base and dashu-int for big
  numbers, parking_lot, rand, toml and its parts, rustix, miniz_oxide and others). Use this feature, always.
- klatch's own `proto/deny.toml` run over that tree: **advisories, bans, licences and sources all pass** (the one
  failure was the scratch crate lacking a licence field). `hegeltest` is MIT.
- Cold build of the test binaries: **10 s wall (30 s CPU) with `static-engine`, 28 s wall (99 s CPU) with the default**;
  target directory 430 MB against 349 MB. `hegeltest` needs Rust 1.86, which klatch (edition 2024) already needs.
- The engine crate has 594 `unsafe` occurrences in about 57,000 lines (and `hegeltest` 172): a count, not an audit. It
  is a dev-dependency, so it is never in the shipped binary or the published crate's dependency tree for users.

**Using it.**
- Properties are `#[hegel::test] fn name(tc: TestCase)` with `tc.draw(generator)` and `tc.assume(..)`; `cargo test`
  runs 100 cases by default. Four properties on real klatch code (clean_path is absolute, normalised, idempotent and
  independent of the directory; `scopes_collide` is symmetric and reflexive; `overlaps` is symmetric; the decoders
  never panic and refuse anything that is not a version 1 frame) ran 400 cases in **0.02 s** and passed. They found no
  real bug.
- Friction: `draw` needs a *printable* generator so it can show the counterexample; a `-> impl Generator` helper or a
  `.map` closure is not one (the compiler's messages name the fix: `print_as_call`, `boxed_printable`, or
  `draw_silent`). Drawing plain integers and mapping them to values inside the test is the simple path. Hegel's plain
  `text()` generator is weak for structured input: it never produced a `..` path segment in 5,000 cases.
- Compared with a hand-rolled xorshift loop in the style of `protocol_fuzz.rs`, on two planted bugs in a copy of
  `clean_path` (a `..` at the root kept; a `..` under exactly three directories ignored): Hegel with a structured
  generator failed at case 3 and 9 and **shrank the first to the minimal input `".."`**; the xorshift loop failed at
  try 57 and 1. So time to find is not where Hegel wins when a person picks a good alphabet; shrinking, replay and the
  failure database are. Also learned: a property must be strong enough (bug 2 left no `..` behind, so "no `..` segment"
  could not see it; "appending `/x/..` changes nothing" did).
- Reproducibility: with `seed(Some(7))` three runs gave identical results; a failure prints
  `#[hegel::reproduce_failure("<blob>")]` that replays it from the blob alone (so a CI failure replays locally);
  `.hegel/examples/` is the failure database and replayed the failure in 0.00 s on the next run (it is now gitignored).
  Settings come from the builder or from `HEGEL_TEST_CASES`, `HEGEL_SEED`, `HEGEL_DERANDOMIZE`, `HEGEL_DATABASE`.
- Stateful testing exists (`#[hegel::state_machine]` with `#[rule]`, `#[invariant]` and a `Pool`), which fits the model
  based claims and cursor tests. The documentation and examples were read; no state machine was run yet.

**Go or no-go: go**, as a dev-dependency with these conditions. Exact version pin (`=0.52.6`; the README says breaking
changes may come in beta, and 0.52.6 was released the day before this was written). `static-engine` always. Default
budget of 100 cases in the normal `cargo test`, which adds well under a second; long runs by hand with
`HEGEL_TEST_CASES`, not in CI (CI stays light). Target pure functions and the gateway over a store, not the CLI process
(each case would spawn processes). Nothing a property test passes is called proven.

**Open risks.** Beta software with fast release cadence. 26 more crates in the lockfile for `cargo-deny` to watch, and an
unaudited engine with a lot of `unsafe` (dev-only). Not tested: the Linux build, Intel macOS (section 41 says it builds
from source there), async and multi-process use, and the stateful API.

**Candidate properties (#389), best first.** Pure and cheap: path cleaning (done), scope and path overlap (done), the
decoders never panic (done; the xorshift fuzz stays), label chain dominance is a preorder and an unknown label
dominates nothing, `valid_name`, `valid_kind` and `valid_scope` accept no control characters or newlines for any input,
`norm` and `flat` are idempotent, the recovery code round trips and any single changed character is detected, a sealed
value opens with its key and context and with nothing else (the crypto is opaque), `judge_tool` agrees with a plain
reference implementation. Model based (state machine against an in-memory model, with a fake clock): claims (one holder
per overlapping path in a scope, release only by the holder, expiry frees, a conflict names the holder), read cursors
(each other member's message is delivered once, one's own never, acknowledgement only moves forward), anchored claims
released by the work item's `done`. Not worth it: SQLite or SQLCipher behaviour, whole CLI flows, anything on the real
clock.

### 42.8 Second round: properties for the stateful features (2026-11-15, tickets #601, #602, #603)

Since the two models and four properties of 42.1 to 42.7 were written the code gained claim renewal (#494), parked
state, archive and restore with tombstones, roles and refusal tables, history paging and a hook byte budget. This round
picked the invariants where a generated search beats example tests, wrote each, ran it, and then **planted a bug in the
real code for each property to prove it can fail** (a property that cannot fail is not kept). Same setup as before:
`hegeltest =0.52.6`, `static-engine`, no new dependency.

**What was added** (all in `proto/klatch/tests/properties.rs` and `models.rs`):

| Property | File | What it checks |
|---|---|---|
| `archive_then_restore_is_the_identity_...` | properties | Up to 14 generated messages (unicode and escaped bodies, handoffs, both attributions), two generated cutoffs archived into two files, restored in either order: after every step `seq` is exactly 1..n, tombstones are exactly the messages older than the later cutoff, a live message never changes, no member's `read` (advancing or not) is handed a tombstone, the second archive moves only what the first left, and after the restores `tail` equals the original log (id, sender, ts, body, kind, payload). |
| `claims_agree_with_the_model_...` (extended) | models | The existing claims model gained a `post` rule (activity renews a live claim to its OWN ttl, counted from now, never shorter, never a lapsed claim), a `park` rule (a `parked:` post holds live claims to the return time) and the unpark cut-back of the next ordinary post, a 24 h ttl and a 90 000 s clock step, and an invariant that no listed claim outlives `CLAIM_MAX_SECS`. The model says renewal in one line (`max(expires, now + ttl)`). |
| `the_hook_output_is_always_valid_json_within_its_byte_budget` | properties | Up to 5 rooms (valid names up to 40 characters) of messages, claims, releases, stale asks, backlog and aged claims whose text holds quotes, backslashes, newlines, control characters, emoji and generated text, repeated: the printed text is within `DIGEST_MAX_BYTES`, the JSON envelope is within `HOOK_MAX_BYTES`, `serde_json` parses it and `additionalContext` equals the text, and `render_hook_within` honours any budget from 4000 up. |
| `the_role_layer_decides_every_operation_...` | models | **Exhaustive**, not sampled: 4 roles (none, librarian, admin agent, reviewer) times 16 gateway operations and 33 admin action names (the allowlist, the four floors, every other act the gateway builds, and unknown ones, which an allowlist must refuse), each on a fresh members' gateway. The expected refusal (rule `reviewer`, `read-only`, `admin-scope`, `floors`, or none) comes from a spec table written separately in the test, not from the gateway's lists. |
| `no_sequence_of_operations_by_a_member_ever_changes_...role` | models | Random attempts, role-granting calls included (own and another's, on and off), by each kind of member on a members' gateway: nobody's role changes. |
| `paging_through_tail_and_history_...` | properties | Up to 48 messages from two senders: `tail` pages of a generated size and `history` pages forward concatenate to exactly the full log (no gap, no duplicate), no page is over its limit, `more` equals the messages after the page, `history --last k` is the end of the log, and every member's unread mail is the same before and after (no cursor moved). The page loops are bounded, so a pager that never advances fails instead of hanging. |

**Planted bugs** (500 cases, `HEGEL_SEED=12345`, each reverted; "caught" means the named test failed):

| # | Planted bug in the real code | Caught by |
|---|---|---|
| A1 | restore leaves `kind` as `archived` | archive property |
| A2 | restore drops the payload | archive property |
| A3 | archive cutoff `ts<=` instead of `ts<` | archive property |
| A4 | `read` delivers tombstones (filter removed) | archive property |
| A5 | archive, its count and its tombstone update lose the tombstone filter (a tombstone is archived again) | archive property |
| B1 | renewal revives a lapsed claim (`expires>now-5`) | claims model |
| B2 | renewal can shorten a lease (drops `expires<now+ttl`) | claims model, only after the `park` rule was added: without a park or a liveness lease no lease ever exceeds `now+ttl`, so shortening cannot be seen |
| B3 | renewal to twice the claim's own ttl | claims model |
| B4 | renewal renews other members' claims | claims model |
| B5 | unpark does not cut the lease back | claims model |
| C1 | hook reserves nothing per room (the real bug of #601, reverted) | hook property |
| C2 | hook JSON does not escape quotes | hook property |
| C3 | hook counts characters instead of bytes | hook property |
| D1 | admin allowlist gains a floor act (`cap`) | role table |
| D2 | librarian may claim (`is_query` widened) | role table |
| D3 | non-reviewers may review | role table |
| D4 | an admin agent skips its scope check | role table |
| E1 | `tail` returns `seq >= after` | paging property (after bounding the loop: first run it hung) |
| E2 | history forward page starts one late | paging property |
| E3 | history `more` off by one | paging property |
| E4 | history advances the cursor | paging property |

Every property caught every plant it was matched with; none was dropped. Two of the plants taught something: B2 showed
the first claims model **could not fail** on renewal shortening (so the park rule was added), and E1 showed a
`loop { ... }` over pages turns a bug into a hang (so the loops are bounded).

**Bugs found.** One real, fixed in the same pull request: **#601**, `plan_rooms` subtracted each room's header and trailer
reserve one room at a time, but a room first spent everything left on its claims and asks, so the headers and "more"
lines of the rooms after it were printed on top of the budget (5 rooms with a flood of claims printed 9205 bytes against
8000; Hegel found it at 8016 with 4 rooms). The unit test was red first, then the reserve for all active rooms is taken
before any unit is fitted. One low-priority limit, **#602**: a budget smaller than the reserve of all active rooms (about
2.6 KB for five rooms) is overshot by a few bytes per room; unreachable today because the session start budget leaves
several KB, so the property draws budgets of 4000 and up.

**Not done, and why.** Candidate (6), migration from every older schema version, needs a fixture of each old schema; the
migrations are private and building version N by running migrations 0 to N would test them against themselves. Filed as
**#603** with options. Also not generated: crossing queue and durable tasks (their refusal rules are inside the role
table through `review_next` and `review_verdict`; the queue's own ordering was not modelled), liveness leases (they need
a system probe), and the convention text (a constant: an example test is the right tool).

**Cost.** The gate runs 500 cases of each: the properties binary 5.5 s and the models binary 4.5 s on this machine,
about 10 s together (the hook property is the heaviest at about 3 s). Nothing needed the `--long` mode.

## 43. A second Store, in memory, as a spike (2026-11-14, tickets #73, #74, #485)

Question: is the `Store` trait shaped by SQLite? Method: the 90 store tests that use only the trait became a conformance suite (`src/store/conformance.rs`, #74); a test-only in-memory store (`src/store/memory_spike.rs`) implements only the 11 required methods and runs the whole suite. Result: **44 of 90 pass**. The core passes (post and read with cursors, members, claims with scopes and expiry, tail, head, advance_cursor); the 46 that fail all need optional methods: hook and wake positions, status, sessions, room state, mute, parent, deadlock detection, asks, anchors, credentials, policy. The trait therefore does not leak SQLite for the core, and the optional methods are where a backend has work to do. The finding that matters: those optional methods default to silent success (`Ok(())` or empty), so a backend can skip them and still compile and run (#485). A real second backend is not built; this spike stays test-only.

## 44. Local models for the optional channel-safety judge (2026-11-14, ticket #54, question 19)

Question: which open-source models could sit behind the `Judge` trait (`DESIGN.md` D4, R31, ticket #48), what are their
licences, and which should klatch point at first? Method: model cards, licence texts and the Hugging Face API read on
2026-11-14 (web fetch, no model downloaded or run). Everything below is **REPORT** (read from a vendor page) unless
marked otherwise. Nothing here was run on a laptop or on klatch messages, so no speed or accuracy claim is klatch's own.

### 44.1 The job, so the comparison fits it

The judge sees one candidate message from an agent room and says whether it is unsafe to relay. The three things that
matter are prompt injection, credential leakage and instructions aimed at other agents. It must run locally, be small,
and never be required (R31). Its output feeds the pipeline in D4 (floors, then the judge as a fast gate, then the
reviewer for the uncertain band), so the useful verdict is three-way (allow, uncertain, deny), not a bare score.

The main finding: **no model below was trained for this exact job.** Most safety classifiers target content harms
(violence, hate, sexual content, self-harm), which is not what a coding-agent room leaks. Injection detectors cover the
first item only. Credential leakage is better done by the scrubber and deny list in the floors (exact patterns), and
"instructions aimed at other agents" is a policy a model must be told in words.

### 44.2 Candidates

| Model | Size | Licence | Output | Covers our job | Notes |
|---|---|---|---|---|---|
| Qwen3Guard-Gen (0.6B, 4B, 8B) | 0.75B, 4.4B, 8.2B | Apache-2.0 (HF metadata) | `Safety: Safe`, `Controversial` or `Unsafe`, plus a category | Category list includes `Jailbreak` and `PII`; no explicit injection or agent-directed class | 119 languages stated; Sep 2025 |
| Llama Prompt Guard 2 (22M, 86M) | 22M, 86M backbone | Llama 4 Community Licence (custom) | benign or malicious score | Injection and jailbreak only; "intent to override instructions" | 512-token window; vendor latency on an A100: 22M 19.3 ms, 86M 92.4 ms |
| Granite Guardian 3.3 8B | 8.2B | Apache-2.0 | `<score>` yes or no, optional reasoning | Jailbreak, harm, function-call hallucination, and user-defined criteria | Released 2025-08-01; custom criteria can express agent-directed instructions |
| gpt-oss-safeguard-20b | 21B total, 3.6B active | Apache-2.0 | reasoned verdict against a policy you write | Whatever policy is written, including klatch's three items | Card says it fits 16 GB of VRAM; harmony format required; reasoning effort low, medium or high |
| Llama Guard 3 1B | 1.5B | Llama 3.2 Community Licence | `safe` or `unsafe` plus hazard codes | Content harms (MLCommons taxonomy) | Pruned and quantised variants aimed at mobile |
| Llama Guard 4 12B | 12B | Llama 4 Community Licence | `safe` or `unsafe` plus codes | Content harms; adds "Code Interpreter Abuse" | Multimodal; card says single GPU |
| ShieldGemma (2B, 9B, 27B) | 2.6B and up | Gemma Terms of Use (custom) | Yes probability per policy you write | Four harms (dangerous, harassment, hate, sexual); policy text is editable | English only; "highly sensitive" to policy wording per its card |
| Kev (0.8B, 4B, 9B, 27B) | 0.8B to 27B | Apache-2.0, base models Apache-2.0 | probabilities per typed question | General decision model, not safety trained | 0.8B stated to run on any Apple Silicon Mac; see 44.4 |
| A small general model (for example Qwen3-4B-Instruct-2507) | 4.0B | Apache-2.0 | whatever the klatch prompt asks | Anything, but untuned | The fallback; weakest against being talked round |

### 44.3 Licence notes (read from the texts, not guessed)

- **Apache-2.0** (Qwen3Guard, Granite Guardian, gpt-oss-safeguard, Kev, Qwen3 base): read from the Hugging Face model
  metadata and, for Granite and Kev, the card text. Permissive; the full Apache text was not re-read here.
- **Llama 4 Community Licence** (Prompt Guard 2, Llama Guard 4; effective 2025-04-05): read in full on the Llama Guard 4
  page. A non-exclusive royalty-free licence with conditions: copy of the agreement and a "Notice" attribution line
  must go with anything klatch redistributes; use is bound to Meta's acceptable use policy; a licensee whose products
  had more than 700 million monthly active users in the month before the release date must ask Meta for a licence.
  It is a custom licence, not an OSI open-source licence. Not checked: whether the Llama 4 text carries the EU
  multimodal restriction (the Llama 3.2 text does, for multimodal models only; Prompt Guard 2 is text only, Llama Guard
  4 is multimodal).
- **Llama 3.2 Community Licence** (Llama Guard 3 1B): same shape, "Built with Llama" display and a "Llama" model-name
  prefix for derived models, plus the EU multimodal clause (not relevant to a text model).
- **Gemma Terms of Use** (ShieldGemma): custom. Use must follow Google's Prohibited Use Policy (incorporated by
  reference), and Google reserves the right to restrict use "remotely or otherwise" of its services it believes
  violate the terms. Outputs are not claimed by Google. Pass-along conditions for redistribution were not read in detail.
- Practical point: if klatch only tells a user how to download a model into their own cache, klatch redistributes
  nothing and the licence binds the user. If klatch ever bundles weights or a derivative in its release, the custom
  licences above add notice and attribution duties and Apache-2.0 models are the clean choice. This is a reading of the
  texts, not legal advice; the human should confirm before any bundling.

### 44.4 Kev, as question 19 asked

Kev (Jared Palmer, `jaredpalmer/kev`) is an open, Jev-like family: state in, typed questions in, a probability per
option out in one forward pass. Hub metadata and the repository README both say Apache-2.0, built on Qwen3.5 (0.8B, 4B,
9B base) and Qwen3.8 (27B), published 2026-09-19 to 2026-10-01 (HF creation and update dates). The README says the 0.8B
runs on any Apple Silicon Mac through MLX and the 4B on a 32 GB Mac. It is a **general** decision model: its published
numbers are on decision benchmarks, none on safety or injection. A model that answers "is this message safe to relay?"
from the text it is judging inherits the steering problem already recorded for Jev (section 13.3). It would fit the
trait's typed-answer shape well but is unproven for this job. Not verified: README latency figures are for a hosted
deployment (one example shows 495 ms), not a laptop.

### 44.5 What fits the verdict

- A three-tier label maps straight onto allow, uncertain and deny: Qwen3Guard's Safe, Controversial and Unsafe. Yes or
  no scores (Granite, ShieldGemma, Prompt Guard) need two thresholds chosen by klatch, ideally in the floors.
- Vendor latency is available only for Prompt Guard 2 (A100, 512 tokens). For the 0.6B to 8B generative models no
  latency figure was found on the cards, and no laptop number was measured here. Design the trait for a call that may
  take seconds, with a timeout that fails closed to the queue (as ticket #48 already specifies).
- Input limits: Prompt Guard 2 takes 512 tokens and wants longer text split into segments; the others were not checked.

### 44.6 Recommendation

1. **Ship nothing model-based by default.** Rules-only stays the default (R31). The judge is an opt-in, and klatch
   should not bundle weights.
2. **First local model to try: Qwen3Guard-Gen-0.6B, with 4B as the step up.** Apache-2.0, smallest of the safety
   classifiers with a three-tier verdict that matches the pipeline, and a `Jailbreak` category. Honest limit: its
   categories are content harms, so it will miss credential leakage and agent-directed instructions that look polite.
3. **For the injection item, Prompt Guard 2 (86M, or 22M for speed) is the purpose-built piece**, with the custom
   licence caveat above; it is a second signal, not a replacement for the first.
4. **For the policy that is really ours (injection, credentials, instructions to other agents), a bring-your-own-policy
   model is the better fit:** Granite Guardian 3.3 8B (custom criteria, Apache-2.0) or gpt-oss-safeguard-20b (written
   policy, Apache-2.0, heavier). Put these in the "uncertain band" slot, not the fast gate.
5. **Fallback with no safety model:** a small general instruct model with a klatch prompt (Apache-2.0 Qwen3-4B class).
   Treat it as weakest against adversarial text.
6. **Kev: park it** as a candidate for typed-verdict experiments, not a safety judge, until someone measures it.
7. **Skip** Llama Guard 3 1B and 4 and ShieldGemma for this job: content-harm taxonomies, custom licences, little
   overlap with the three risks.

Whatever is chosen, the same caveat as D4 stands: every model here can be fooled by crafted text, and the floors
(scrubber, deny list, label order) stay the guarantee. Before any model is wired in, build a klatch evaluation set of
real room messages with injected, credential-bearing and agent-directed examples, and run candidates on it.

### 44.7 Not verified

No model was downloaded or run. Memory, quantised size and laptop latency for every model except the vendor lines
quoted. Whether Granite Guardian or gpt-oss-safeguard follows a klatch-specific policy reliably. Llama 4 licence EU
clause and ShieldGemma redistribution terms in full. The Apache-2.0 text itself. Kev's behaviour on safety questions.
Protect AI's deberta injection classifier was seen but its repository is marked archived, so it was left out. Models
released after 2026-11-14 are not covered.

### 44.8 Sources (all read 2026-11-14)

Hugging Face model cards: `meta-llama/Llama-Guard-4-12B`, `meta-llama/Llama-Guard-3-1B`,
`meta-llama/Llama-Prompt-Guard-2-86M`, `google/shieldgemma-2b`, `ibm-granite/granite-guardian-3.3-8b`,
`Qwen/Qwen3Guard-Gen-0.6B`, `openai/gpt-oss-safeguard-20b`, `jaredpalmer/kev-0.8b` to `kev-27b`; the Hugging Face
model API for licence and parameter counts; <https://ai.google.dev/gemma/terms>;
<https://github.com/jaredpalmer/kev> README. A third-party summary of Kev (web search) was used only to find the
repository, and every Kev fact above was re-read from the repository or the Hub.

## 46. The liveness probe on real sessions (2026-11-15, Claude Code 2.1.325, macOS, ticket #310)

`cargo test -p klatch --test liveness_real -- --ignored --nocapture` probes every live session in `~/.claude/sessions` (read only: registry, `ps`, file metadata) twice, five seconds apart.

- **OBSERVED.** Five live interactive sessions: every layer was readable for all five (process state, cpu, children, turn, transcript; task output absent as expected). The one busy session (the one running the test) read `Working`, with transcript activity within the second and CPU growth between samples. The four idle sessions read `WaitingOnHuman`, status age 124 s at the first sample and no CPU growth. Verdicts matched what was known of each session.
- **COST.** 33 to 43 ms per probe on a 5-session machine, nearly all of it the single `ps -A` call. A hook that probes N holders spawns N `ps` calls, so a shared table read per pass would cut it (not done).
- **FOUND.** Idle sessions had 3 to 6 live descendants and the busy one 11 (MCP servers and helpers started with the session). A child count above zero therefore says little about a running tool: a hung busy session would read `WaitingOnTool` until the lease cap, never `Stuck` (ticket #538).
- **NOT TESTED.** A deliberately hung tool call, a long silent build, a sleeping tool call, a background-blocked session, a killed real session: the machine's sessions were all in use and none was to be disturbed. The killed case is covered with fake sessions in the bench (`a_dead_holders_long_claim_lapses_after_the_grace_period`); the others only by unit tests on made-up tables.

## 47. The inbound gate, the hold and refuse paths, and asyncRewake in the terminal, run live (2026-11-15, Claude Code 2.1.319 to 2.1.325, macOS, Darwin 23.3.0, tickets #85, #86, #87)

Scripts: `experiments/gates/` (`recv.py` headless receiver with a raw-socket or real-session sender, `tui.py` an interactive receiver in a tmux pty, `post.py`, `wake.py`, `m87a.sh`, `m87b.sh`, `m86tui.sh`, `m87tui.sh`).

**Method.** One receiver per case in a scratch directory, started with a throwaway `--settings` file and `--setting-sources local`; the user's own login (an isolated `CLAUDE_CONFIG_DIR` starts logged out); the five installed binaries run by path from `~/.local/share/claude/versions` (2.1.319, 2.1.321, 2.1.322, 2.1.323, 2.1.325), so a version is a binary and not a guess. Headless receivers: `claude -p --input-format stream-json --output-format stream-json --verbose --model haiku --permission-mode M`. Terminal receivers: the same binary in a tmux pane (a real pty) on a private tmux server (`tmux -L gates`) started with the shell's environment minus Claude Code's own `CLAUDE_CODE_*` variables (a plain `env -i` shell was logged out). A sender is either a Python process that is not the receiver's child, writing to `messagingSocketPath` from the registry record, or a second real session (`claude -p`, permission mode S, tools `ListAgents` and `SendMessage`). The test text asks for a bare marker word; "acted" means the receiver's own reply is exactly that marker (in the terminal: a line `⏺ <marker>`, because the held-message notice also contains the marker text and an earlier count-based check was wrong because of it). The receiver's hold approval deadline was shortened with the environment variable `CLAUDE_CODE_USER_DIALOG_TIMEOUT_MS=8000` for the headless runs. No klatch binary, store or Keychain was involved: these cases test Claude Code only. Side effect: every session wrote a transcript under the user's `~/.claude/projects` for its scratch directory. `bypassPermissions` was used only for receivers and senders whose scratch directory was empty and whose task was a bare reply, `ListAgents` and `SendMessage`, or a foreground `python3 time.sleep`.

### 47.1 Which gate applies (question 29, #86)

**OBSERVED, five versions, headless receiver, default mode, idle.** A message with no auth line, with a wrong token, and with the right token (read from the key file) was acted on every time: 15 of 15 (5 versions by 3). **Terminal receiver:** wrong token, default mode, 2.1.325 and 2.1.319: acted on in about 2 seconds. Wrong token, `bypassPermissions` with `crossSessionInbound: accept`, 2.1.325: acted on. A busy receiver is covered in 47.2.

**READ FROM THE BINARIES (`strings` on all five).** The socket server sets `authRequired = requireAuth ?? (platform === "windows")`; the only reference to `requireAuth` in each binary is that parameter, no caller passes it, and the string is identical in 2.1.319, 2.1.321, 2.1.322, 2.1.323 and 2.1.325. So on macOS (and, by the same expression, Linux) the token is **not enforced in any version tested**; on Windows it is. The token check exists (an `auth` line with a bad token destroys the connection when `authRequired` is true), it is just off. The other gates that remain: the socket directory and file modes (0700 and 0600, so another user cannot connect), the same-user process that reads the registry, and the receive-side policy of 47.2. The policy source is the same in all five binaries (same strings, same cause names).

**Answer.** On macOS, any process of the same user can post to any session whose registry record it can read, with or without the token, on every version tested, idle or busy, prompting or bypassing; the receive-side hold is the only thing between that post and the model, and it applies only when the receiver bypasses permissions or has set `crossSessionInbound`. **Recommendation:** keep the 14.2 position and make it a rule (an invariant for tests): klatch never relies on the inbound token or on the socket for security; a nudge is content-free (R18); re-run the static check (`requireAuth`) on each new Claude Code version before trusting any statement about the token in either direction.

### 47.2 The hold and refuse paths with mixed permission modes (questions 8 and 14, #87)

**READ FROM THE BINARY (2.1.325).** Policy order: a setting `crossSessionInbound` (`accept`, `hold`, `refuse`) from managed, then `--settings`, then user settings wins; project and local settings may only tighten (`accept` there does not loosen). A value that is not one of the three is held. With no setting: the receiver's mode is classed `bypass` (`bypassPermissions`, or a mode that is bypass-like when bypass is available) or `prompting` (everything else); an unrecognised or unreadable mode holds. A sender's mode is compared only if the message carries `fromMode`; the schema says that is "honored only from the injecting host on local stdin", and the socket line parser has no such field. Hence, for a **raw socket sender**, which cannot attest a mode: a prompting receiver accepts, a bypassing receiver holds (`no-mode-asserted`). For a **real sender session** (`SendMessage`), the harness attests: equal class accepts, different class holds (`mode-mismatch`). A held message is parked (at most 100); if the cause is a mode cause an approval dialog opens in the terminal; no answer by the deadline drops it with outcome `expired` (default deadline 300000 ms, set by the environment variable above or `dialogExpiry` 60s, 5m, 10m, never); a headless session has no dialog, so every mode hold there ends `expired`.

**OBSERVED, raw socket sender, headless receiver (`m87a.sh`, 2.1.325; one post per cell):**

| Receiver mode | no setting | `accept` | `hold` | `refuse` |
|---|---|---|---|---|
| default | acted | acted | held (`explicit-setting`), not delivered | dropped silently, no event |
| acceptEdits | acted | acted | held (`explicit-setting`) | dropped silently |
| plan | acted | acted | held (`explicit-setting`) | dropped silently |
| bypassPermissions | **held (`no-mode-asserted`), dropped `expired` 8.0 s later** | acted | held (`explicit-setting`) | dropped silently |

`auto` and `dontAsk` receivers, no setting: both acted (classed prompting). The stream carries a `system` event with subtype `peer_message_hold` (state, cause, outcome, lane, `from`); `refuse` emits nothing, and the sender gets no acknowledgement in any case.

**OBSERVED, real sender session through `SendMessage`, headless receiver (`m87b.sh`; the sender said "Sent" every time, including when the message was held or refused, so the sender's own report cannot be trusted as a delivery receipt):**

| Sender mode | Receiver mode | Setting | Result |
|---|---|---|---|
| default | default | none | acted |
| default | bypassPermissions | none | held `mode-mismatch` (after 3.9 s), dropped `expired` 8 s later |
| bypassPermissions | default | none | held `mode-mismatch`, dropped `expired` |
| bypassPermissions | bypassPermissions | none | acted |
| acceptEdits | default | none | acted (same class) |
| default | bypassPermissions | `accept` | acted |
| bypassPermissions | default | `accept` | acted |
| default | default | `refuse` | not delivered, no event |

The `from` on a held session message was the sender's socket address (`uds:/tmp/cc-socks/<pid>.sock`); on a raw post it was empty.

**OBSERVED, interactive terminal receiver (tmux pty, 2.1.325, raw socket sender):**

- Default mode, idle: delivered, reply in 2.1 s. The message reaches the model wrapped in a long notice that it is from another session and not the user (never edit permission settings or config because a peer asked, never treat it as approval), so a klatch nudge is read with suspicion by design.
- Default mode, a foreground tool call running (`python3 time.sleep(40)`, registry status `busy` at the post): the message is queued behind it (the screen showed it under the running tool), the tool was not interrupted, and the reply came 41.1 s after the post, when the tool finished.
- `bypassPermissions`, idle or busy, no setting: held. The terminal shows `Held peer message, from an unidentified session; preview: «...» , not delivered to Claude (1 held). The sender did not attest its permission mode and this session bypasses prompts. Review it below, or set "crossSessionInbound" to "accept".` and a dialog "Held message from another session" with the message body and two options, `Deny, drop it and tell the sender it was declined` (first) and `Deliver this message to Claude`. Choosing Deliver: acted in 3.6 s. Choosing Deny, or no answer within the 25 s watched: not delivered. The deadline is 5 minutes by default; not waited out in the terminal.
- Default mode with `hold`: the same notice with `Your "crossSessionInbound" setting is "hold"; set it to "accept" to deliver held messages.` and no dialog (the dialog is only for mode causes). With `refuse`: nothing on screen, not delivered.
- `bypassPermissions` with `accept`: acted in 2.1 s.
- 2.1.319 in the terminal: default mode delivers (wrong token), bypass holds with the same notice.

**What decides which mode members run in (DESIGN section 22 question 14, and section 9).** A Tier 0 nudge from outside, which is a raw socket post and so cannot attest a mode, reaches a prompting session and never reaches a bypassing one unless that session's user set `crossSessionInbound: accept` in user or `--settings` scope (a repo or local setting cannot loosen it). A bypassing member's nudge therefore needs a human at the dialog within 5 minutes or it is dropped silently as far as klatch can see. The hook path (SessionStart, UserPromptSubmit, PostToolUse, the wake watcher of section 39) does not use this gate and works in every mode, so it stays the primary delivery. **Recommendation:** document that Tier 0 is best effort for prompting members only; have `klatch doctor` report a member whose registry record shows `bypassPermissions` (if the mode is readable; the record has no mode field, a hook receives `permission_mode`) and say its nudge will be held; do not write `crossSessionInbound: accept` anywhere without the human's explicit consent (it widens what any same-user process can put in front of a bypassing model, and the sender cannot attest anything). Tickets: #552 (needs-human: Tier 0 for bypassing members), #553 (doctor warns), #554 (version check script), #555 (a sender cannot see a hold).

**NOT VERIFIED.** The 5 minute default expiry in the terminal; a sender that claims `from_name`; whether `SendMessage` from a terminal sender attests the same way as a headless one (the harness code is shared); modes other than the six named; Linux and Windows (the token result is read from the binary, not run); versions before 2.1.319 or after 2.1.325; the effect of `isolatePeerMachines` and managed settings.

### 47.3 asyncRewake in the interactive terminal and under long delays (questions 3 and 30, #85)

Method: `wake.py`: a `SessionStart` hook, `async` and `asyncRewake`, `sleep DELAY; date +%s > stamp; echo "reply with exactly the word WKxxxx" >&2; exit 2`, in a tmux pty (2.1.325, Haiku, default mode, scratch directory, no klatch). The session answers one prompt first, so it is idle when the hook ends. Latency is the time from the stamp file to the marker on screen, polled every 2 seconds (so up to 2 s of it is the poll).

| Case | Result |
|---|---|
| delay 30 s, timeout 120 | woke; the screen shows `Stop hook feedback` and the model's reply with the marker; the hook's stderr text itself is not printed |
| delay 700 s (past the 600 s default), timeout 900 | woke 2.6 s after the hook ended (idle for about 11 minutes) |
| delay 45 s, a foreground tool call (`python3 time.sleep(25)`) running when the hook ends | the tool was not interrupted; the wake came 22.8 s later, when the tool finished, as one extra turn |
| delay 60 s, timeout 30 s | the hook was killed at its timeout: no wake, no message on screen, nothing for the user to see |
| delay 3660 s, timeout 3800 s | woke 2.3 s after the hook ended, after 61 minutes idle (the `Stop` watcher from the first turn was the only thing running; no keepalive) |

So a hook-started watcher can wake an idle terminal session at any delay below its hook `timeout`; the timeout, not the 600 s default, is the limit (3660 s measured); a watcher that outlives its timeout dies silently, so a watcher must exit before its timeout and be re-armed by the `Stop` hook (this is what klatch does, section 39). Earlier runs (section 39) covered the terminal with the klatch watcher, a second session, and background shells; this one adds the delay, the timeout kill and the hour.

### 47.4 What this changes

- DESIGN question 29 is answered: no gate from the token on this platform, on any version tested; question 8 and 14 are answered as in the tables; question 30 loses its asyncRewake clause (PostToolUse and PreToolUse are in sections 21 and 48).
- A held or refused message looks successful to its sender in every path: a sender cannot learn that a nudge was held. Delivery confirmation has to come from klatch (the reader's cursor), never from the socket.

## 48. Hook output, nudge latency and detached watchers, run live (2026-11-15, Claude Code 2.1.325, macOS, Darwin 23.3.0, tickets #83, #84, #88, #89)

Method: headless `claude -p --model haiku --setting-sources local` with a throwaway `--settings` file per case and `--include-hook-events --output-format stream-json --verbose` (the stream carries each hook's raw stdout, stderr and exit code). Scratch directory only; the klatch case used the binary built in a worktree with `KLATCH_HOME` and `KLATCH_KEY_DIR` pointing at a temp directory (no Keychain, no live store). The real login was used. Each result is OBSERVED on one version and one machine. Scripts are not committed; the commands are described per case.

### 48.1 PreToolUse: warn without blocking (#84)

Five hooks on `PreToolUse` with matcher `Bash`, all exit 0, run on one `echo hi`; the model was asked to list every `PRE-` token it saw outside the prompt. Two runs.

| Hook output | Reached the model? | Notes |
|---|---|---|
| plain stdout, exit 0 | no | not in the model's answer in either run |
| stderr, exit 0 | no | same |
| JSON `hookSpecificOutput.additionalContext` | **yes** | quoted in both runs |
| JSON `permissionDecision: "allow"` with `permissionDecisionReason` | no | the reason is for the user |
| JSON `systemMessage` | no | printed to the user as `PreToolUse:Bash says: ...` (a stream event of level notice) |

The command ran in every case (the tool result `hi` came back), so a warning does not block. Conclusion: to warn the model on `PreToolUse` without blocking, print JSON with `additionalContext`; plain text works on `SessionStart` and `UserPromptSubmit` only. Exit 2 (blocking) is in 14.1. Not tested: `permissionDecision: "ask"` (no human in a headless run).

### 48.2 Output limits (#84)

A hook printed numbered 50 byte lines then an end marker; the model reported the highest line and whether the marker was visible, and quoted any notice.

- **Limit.** Whole text delivered at 9000, 9300, 9500 and 9700 bytes. At 9900 bytes and above (10000, 11000, 30000, 100000, 500000 tested) the model gets this instead: a notice `Output too large (<size>). Full output saved to: ~/.claude/projects/<project>/<session>/tool-results/hook-<id>...txt` and a preview of the first 2 KB (about 39 lines). So the cap is between 9.7 and 9.9 KB (the documented figure is 10,000 characters; the gap is probably a wrapper).
- **Not silent.** The notice appears; the model can read the file with Bash (it did in several runs, one turn more) and could not without a tool.
- **Same cap everywhere.** `SessionStart`, `UserPromptSubmit` (plain), `PreToolUse` and `PostToolUse` (JSON `additionalContext`) all behaved the same at 11 KB and 100 KB.
- **Klatch exceeds it.** 15 posts of about 900 characters, the real `klatch hook --event UserPromptSubmit`: stdout 14296 bytes; the model saw the first message only and quoted `Output too large (13.9KB)`. The digest limit is 20 messages by 1000 characters (about 20 KB). Ticket #549.

### 48.3 PostToolUse injection on 2.1.325 (#83)

Recorded in section 21 (2.1.323) and confirmed interactively in #199. Repeated here on 2.1.325: two runs with plain stdout, stderr and JSON `additionalContext` on `PostToolUse`. Only the JSON token reached the model, both runs. #83 is complete; no change.

### 48.4 Nudge size and latency (#88)

A Python process (not a child of the session) connected to the session's `messagingSocketPath` from its registry record and wrote one line `{"type":"user","message":{...},"priority":"now","uuid":...}` with no auth line (not needed in this run; see #86). The session was `claude -p --input-format stream-json --replay-user-messages`. Times are from the send to the model's reply.

- **Idle peer, latency.** 94 bytes: 1.1 to 2.3 s (first turn after a warm-up 1.9 to 2.3 s, later 1.1 to 1.3 s). 1 KB: 1.3 s. 10 KB: 1.3 s. 100 KB: 2.0 s. 300 KB: 5.3 s. This is model time; the socket write took 1 to 6 ms and returns no acknowledgement. The wake-watcher path (hook, section 39) measured 1.7 s to a new turn, the same order.
- **Size.** Delivered intact up to 300 KB (the model counted the last numbered line correctly: 198, 1998, 5998). Claude Code wraps the text as `Another Claude session sent a message: <text>`, about 580 characters of overhead, and shows no sender. Over 300 KB was not tried (about 75k tokens each; documented limit about 1,000,000 characters).
- **Busy peer.** A foreground tool call (python sleeping 25 s; a plain `sleep` is blocked by the harness and runs in the background, so it is not a busy test). The nudge sent 5 s in was not delivered during the tool: it was processed 1.7 s after the tool returned (22.6 s after the post) as a turn of its own, and the model answered it and the original task. So a busy peer sees the nudge at the next idle moment, late by the remaining tool time; this agrees with the 39 result for the hook path.
- **A bug in the plan (ticket #550).** Six identical 94 byte posts about 2.5 s apart to an idle session: only the first and the fourth produced a turn; the others were dropped with no error and no event. The same six with a changing nonce at 1 s and at 4 s gaps: all six delivered. Each had a distinct uuid, so the drop keys on the text. A constant nudge text can vanish; include the unread count or last seq.
- **Recommendation.** Keep nudges short (under 1 KB; the nudge carries a pointer, the content is read from klatch), never repeat identical text, and treat the wake as late by up to the length of the current tool call. Not measured: the interactive terminal, and `priority` values other than `now`.

### 48.5 Does a watcher survive the hook timeout? (#89)

A `SessionStart` hook started a watcher that logged one line a second for 45 s with its parent and ancestor chain; the session ran one 12 s tool call (about 16 to 18 s in all). Five cases:

| Case | Hook | Watcher | Result |
|---|---|---|---|
| a | sync, default timeout, returns at once | background child, same process group | **survived** all 45 s, past the end of the session; parent became pid 1 straight away (not a descendant of claude) |
| b | sync, timeout 2, hook waits 10 s | same process group | **killed** at the timeout: 2 heartbeats; the hook never finished |
| c | `async` + `asyncRewake`, timeout 3 | the watcher is the hook (foreground) | **killed** at 3 s: 3 heartbeats |
| d | sync, timeout 2, hook waits 10 s | child in its own session (`start_new_session`) | **survived** all 45 s, past session exit; parent pid 1 |
| e | `async` + `asyncRewake`, timeout 3, hook waits 30 s | same process group | **killed** at 3 s |

Findings. (1) A hook timeout kills the hook's whole process group, so a child that stays in it dies with the hook. (2) A child in its own session survives the timeout and the end of the session, but it is then an orphan of pid 1: it is no longer an own child of the session, so the documented own-child exemption for posting into the session's socket cannot apply to it, and it cannot wake the session by exit code (that needs a live hook process). While the hook is alive the chain does reach claude (`sh, sh, claude`). (3) A watcher that is not detached ends at the hook timeout, so the wake path must either raise the timeout (section 39: 900 s worked, 660 s idle then woke in 2.5 s) or re-arm from a `Stop` hook (section 39, done). A detached watcher would outlive its session; it must end itself (klatch wake has its own timeout and a lock, and the member's `SessionEnd` is not a kill signal for it).
- **Recommendation.** Do not detach. Keep the foreground `asyncRewake` watcher with a hook timeout above the watcher's own timeout, and re-arm on `Stop`. Not tested: interactive TUI, an `async` hook without `asyncRewake`, a longer hold than 45 s, `kill -9` of the session (an orphaned detached child would remain).

### 48.6 Tickets

#549 (hook digest can exceed the output cap), #550 (identical nudge text dropped). #83, #84, #88 and #89 have their result here.

## 49. Cotal and agent-loom read in full, Codex queue and hooks, the channels preview, native peers (2026-11-15, Claude Code 2.1.325, Codex 0.147.0 installed, tickets #90 to #93)

Sources were fetched on 2026-11-15 and are named per item. Each item says what was read, what was run and what could not be verified. Licence terms are quoted from the repository only where the page states them; nothing here is legal advice.

### 49.1 Cotal (ticket #90, question 15)

Read: `https://docs.cotal.ai` (home, `llms.txt`), the Identity page (`/docs/identity-and-auth.md`), Connect Claude (`/docs/connect-claude.md`), Connect Codex (`/docs/connect-codex.md`). Not read: the wire specification (SPEC sections 3, 5, 9, 10), the connector reference pages, the source code.

- **DOCUMENTED, identity.** A principal is an `owner.actor` pair; open development meshes use owner `local`. The broker pins sender tokens in publish subjects to the connection's authenticated principal, so a peer cannot publish as another owner or a sibling actor, and receivers must also reject a payload whose `from.id` differs from the subject sender. `cotal up` creates a space with JWT authentication; `cotal up --open` is an unauthenticated development mesh and "is outside the security guarantees in SPEC section 9". Both bind to loopback by default. So on the default path sender identity is enforced by the broker (a network service), not inferred from the process tree. Klatch's equivalent is the gateway-owned socket plus peer pid (section 38), which is weaker on a shared machine and needs no daemon-held credentials.
- **DOCUMENTED, joining a session Cotal did not start.** Through the hosted connector or the native plugin: the conversation calls `cotal_connect` with a name, a human opens an approval page in a signed-in browser, inspects the space and permissions and approves; "A pending request has not joined." A held name stays reserved: "No rename or local deletion proves that authority ended." This is a human-approved enrolment where klatch's auto-join at SessionStart (the init consent, DESIGN D-section on auto-join) trades approval for friction.
- **DOCUMENTED, labels.** The Identity page has capabilities (`spawn`, `run`) for control authority and channel `allowSubscribe` and `allowPublish` for data. No sensitivity label, clearance or scrubber appears on any page read. The survey's differentiator claim (SURVEY 1.4) still holds for these pages; the wire spec was not read.
- **DOCUMENTED, wake and expiry.** Native Claude Code delivery uses supported hooks; hosted access "cannot start a turn in an idle conversation". When channels are blocked, "An incoming message cannot wake an idle managed session", and Cotal says to check the startup notice before leaving an agent unattended. Its own tooling flags the state: a `cotal spawn` receipt carries `STARTUP_NOTICE`, `cotal ps` marks the seat `no idle wake`, and a `once` launch of such a seat is refused. Expiry of anything other than connection requests and held names was not found on the pages read (presence is a heartbeat; section 3.2 of SURVEY records "liveness, not proof that a task is progressing").
- **DOCUMENTED, Codex.** The Codex connector runs `codex app-server` and attaches the TUI to the same thread; each agent gets a private `CODEX_HOME` with only `auth.json` linked, so the user's hooks, MCP servers and configuration do not load. Peer messages start turns; "Session resume and extra MCP servers are unsupported" (beta). It delivers through the app server, not hooks.
- **NOT VERIFIED.** Whether a message can be injected without a broker credential on an authenticated mesh (the pages say no; not tested). Licence: Apache-2.0 was recorded from the repository page on 2026-11-12 (SURVEY 3.2) and was not re-read today. The README telemetry default (usage telemetry on unless `cotal telemetry off`) is from SURVEY 3.2.

Answer to question 15: identity is broker-enforced with a signed-in or local credential; there is no label concept; an idle native session is woken through channels only, and Cotal states plainly when it cannot be.

### 49.2 agent-loom (ticket #90, question 16)

Read: the full README at `https://github.com/osteele/agent-loom` (MIT per its License section, 198 commits at fetch time). Not read: `docs/architecture.md`, `specs/`, the source.

- **DOCUMENTED, identity.** A sender is attributed only when agent-loom proves "that the registered session's host process is an ancestor of the CLI process"; the stored session id and source project are used, and "Sharing a project or matching a display name is insufficient." An unattributed read "reports the condition on stderr and leaves the mail unread." From the survey: a free-form `--from` is stamped `[label; not a reply address]`. What a receiving model sees for that label was not found on the README. This is the same evidence class as klatch's ancestry walk (section 17), same-user and process-tree based.
- **DOCUMENTED, expiry.** Path claims are advisory; "Session claims survive a disconnect for 15 minutes." A new claim returns a one-time release token. There is no per-claim TTL on the README: instead the daemon sends reminders to the exact owning session when a claim reaches an age milestone, "any claim after 2 and 8 hours, and then daily starting at 24 hours" (a materialised experiment reservation after 15 minutes); work leases "carry explicit activity". Forced recovery needs user-supplied `authority` and `reason`, both audited. Answer to question 16, first part: claims do not expire by clock; they lapse 15 minutes after the owner disconnects, and are nagged when old.
- **DOCUMENTED, push.** Channel push into Claude Code takes three parts: the plugin installed, managed settings with `channelsEnabled: true` and an `allowedChannelPlugins` entry, and each session launched with `claude --channels=plugin:agent-loom@osteele-local`. A session launched without it is tagged `{channel:host-not-loaded}` in `agent-loom listeners`. Behaviour under `-p` is not on the README; per the Claude Code docs (49.4) the development flag is ignored under `-p`, and an allowlisted channel under `-p` was not found documented. Without push, the "reminder hooks can inject the unread count first" and the session reads on its next `check_inbox`.
- **DOCUMENTED, delivery.** Receipts distinguish "spooled, pending, held, pushed, push-unreachable, read, refused, and expired". Per-session `hold` and `refuse` inbound policies delay or suppress entry into context. Mail is "explicitly marked untrusted and cannot approve permissions". The daemon binds 127.0.0.1, so "any process running as the local user can submit text".
- **DOCUMENTED, harnesses.** Claude Code, Antigravity CLI, Codex and any MCP client use the same tools and spools; push is Claude Code only.
- **Borrowed or not.** Worth borrowing: per-message receipt states (klatch's `post` says only "queued; not proof of read"); age-milestone reminders to a long-held claim's owner instead of silent lapse. Not borrowed: a daemon on a TCP port.

### 49.3 Codex queue and Codex hooks (ticket #91, questions 21 and 22)

Sources: `https://developers.openai.com/codex/cli/reference.md` (the `codex queue` section), `https://developers.openai.com/codex/hooks`, `codex --help` and `codex exec --help` on the local binary. The npm registry reports `@openai/codex` 0.169.0 (modified 2026-11-14); the installed binary is 0.147.0.

**Question 21, `codex queue`.**
- **DOCUMENTED (OpenAI CLI reference, read 2026-11-15).** `codex queue --thread <SESSION_ID> --message "..."` queues a text message for an existing session. `--thread` takes a UUID or an exact session name ("Use a UUID when sessions share the same name"). Queued messages use the session's model; text only. `--resume` queues and then resumes the session if not loaded, which needs a running local daemon or a server selected with `--remote` (and `--remote-auth-token-env`); if resume fails "it prints a warning and still exits successfully".
- **Answers.** Addressing: id or exact name (klatch must use the id, question 26). Sender attribution: none documented (the command takes only a thread and a message). Acknowledgement: none documented; the resume failure case exits 0, so success is not proof. How the receiver treats it: only "queued for the next turn" semantics, nothing about permission classes or trust; the Codex TUI separately documents Tab to queue a follow-up for the next turn.
- **OBSERVED.** The installed 0.147.0 has no `queue` subcommand: its command list (`exec`, `review`, `resume`, `fork` and others) has no `queue`, and `codex queue --help` prints the top-level help, which treats the words as a prompt. The earlier survey line "v0.149.0" is consistent with that. The introducing version was not found (the OpenAI changelog page returned no `queue` line); this was not run on a newer binary.
- **NOT VERIFIED.** Behaviour on 0.169.0 or any version with the command; whether queue reaches a session that is mid-turn or idle without a daemon.
- **Consequence for the adapter.** A wake path via `codex queue` must be probed by running `codex queue --help` for a real subcommand (not by version number) and must send content-free text, since there is no sender field and the text becomes a user turn.

**Question 22, hooks in `codex exec`.**
- **DOCUMENTED.** "Before a non-managed hook can run, Codex requires you to review and trust the exact hook definition. Codex records trust against the hook's current hash, so new or changed hooks are marked for review and skipped until trusted." Managed hooks (system, MDM, cloud, `requirements.toml`) are trusted by policy. `--dangerously-bypass-hook-trust` runs enabled hooks "without requiring persisted hook trust for that invocation" and is "Intended only for automation that already vets hook sources". Project-local hooks load only when the project `.codex/` layer is trusted. The page does not say whether hooks run in `codex exec`.
- **OBSERVED (Codex 0.147.0, 2026-11-15, temp `CODEX_HOME` holding only a `hooks.json` with `SessionStart` and `UserPromptSubmit` command hooks that append a line to a log; no login, so the model call failed with 401 after the hooks stage; no klatch binary, no keychain, no live store).**
  - With no trust and no flag, `codex exec --json "say ok"`: the hook log stayed empty and the JSON event stream and stderr contained no mention of hooks. So an untrusted hook is skipped silently in an unattended run. The interactive-startup warning the page describes ("prints a warning that tells you to open /hooks") was not seen in `exec`.
  - With `--dangerously-bypass-hook-trust`: both hooks ran; each received JSON on standard input with `session_id` (equal to the `thread_id` in the stream), `transcript_path`, `cwd`, `turn_id` (the prompt hook), and the prompt hook ran once per turn. The stream carried the warning "`--dangerously-bypass-hook-trust` is enabled. Enabled hooks may run without review for this invocation." (printed twice).
  - A user config records persisted trust as `[hooks.state."<hooks file>:<event>:<n>:<m>"]` with `trusted_hash = "sha256:..."` (seen in the structure of the local `~/.codex/config.toml`; the hash input was not determined, so klatch cannot write it and should not).
- **NOT VERIFIED.** Whether plain stdout from a hook reaches the model in `exec` (the 401 stopped the turn before any model output; the docs say plain text on stdout of `UserPromptSubmit` is added as developer context). Whether `exec` honours hooks in a trusted project layer. The effect of `exec --ignore-user-config` on user-level hooks was not run.
- **Consequence.** A Codex adapter that installs hooks must tell the human to trust them in `/hooks` (DESIGN question 10) and must verify by a check that works unattended; klatch must not pass `--dangerously-bypass-hook-trust` itself. A doctor probe can read the config for the hook entries and report "not trusted, will be skipped silently" (ticket filed).

### 49.4 Channels preview status (ticket #92, question 10)

Sources: `https://code.claude.com/docs/en/channels` and `/channels-reference` (read 2026-11-15), and the public Claude Code `CHANGELOG.md` on `main` (top entry 2.1.325, the installed version).

- **DOCUMENTED, status.** Still "a research preview"; "Availability is rolling out gradually, and the `--channels` flag syntax and protocol contract may change based on feedback." Neither `--channels` nor `--dangerously-load-development-channels` is listed in `claude --help`; "The flags work even though they aren't listed."
- **DOCUMENTED, constraints.** Anthropic authentication (claude.ai or a Console API key); not on Bedrock, Google Cloud's Agent Platform or Foundry; Team and Enterprise must enable (`channelsEnabled`). `--channels` accepts only the Anthropic allowlist (the plugins in `claude-plugins-official`: Telegram, Discord, iMessage, fakechat) or the organisation's `allowedChannelPlugins`; anything else "starts normally but the channel doesn't register". A custom channel needs `--dangerously-load-development-channels` (per entry, confirmation prompt, not combined with `--channels` entries). "If you pass it in non-interactive mode with `-p` or through the Agent SDK, Claude Code ignores the flag and the channel doesn't register." An empty `allowedChannelPlugins` does not block the development flag; only leaving `channelsEnabled` unset does.
- **Answer to question 10.** No: as of 2026-11-15 a custom channel still needs the development flag and does not work under `-p`. No self-service route onto the allowlist is documented.
- **From the changelog (V-PAGE).** Added in 2.1.80 (`--channels`, research preview); permission relay 2.1.81; `AskUserQuestion` and plan mode disabled under `--channels` 2.1.83; `allowedChannelPlugins` 2.1.84; inbound notifications "silently dropped after the first message for Team/Enterprise users" fixed 2.1.105; Console (API key) auth 2.1.128; connections dropping after the agents view, `/bg`, `/tui` or `/update` fixed 2.1.187; `allowedChannelPlugins` unreadable now admits nothing 2.1.267; plugin entries must match name as well as marketplace 2.1.281. A feature that has been in preview for roughly 245 versions and still carries these fixes is not a stable base.
- **Not found.** An explicit statement that an idle session wakes on a channel event; Cotal's docs (49.1) say channels are what wakes an idle managed session and warn that it is not guaranteed, which is vendor evidence, not a Claude Code statement. Not tested: a channel on this machine (no channel built; klatch's own wake is the hook watcher, section 39).
- **Consequence.** Keep channels an optional accelerator (R18) behind a probe; nothing in klatch may depend on it. The hook watcher stays the wake path for the headless and interactive cases already measured.

### 49.5 Native peer registration (ticket #93, question 27): decision DROP

Evidence from the earlier attempts (section 14.3): two forged registry records (the second with a valid session id and a key file named by the SHA-256 of the token) were never listed by `ListAgents`, and `SendMessage` reported the name unreachable; the server did `LOCAL_PEERPID` on the connecting process. Today's documentation adds (cross-session messaging page, read 2026-11-15): a session "appears only when it binds an inbox socket"; the socket is restricted to the owning OS user; on Linux, Claude Code verifies senders "by process evidence even for a child that has already exited", on macOS only while the poster is running, and a message it cannot verify "is treated like any other that asserts no permission class", which a bypassing session holds for approval. Nothing on the page documents registering a non-session peer.

**Decision: drop native-peer mode. Do not pursue unless Anthropic documents a registration route.**

Reasons:
1. Two failed attempts, and the page now says why the check is stronger than the file: process evidence and the bound socket. Passing it would mean imitating an undocumented handshake (against the minimally invasive maxim) that may change in any release.
2. Registering as a peer would make klatch appear as a Claude Code session to the human, mixing klatch rooms into `ListAgents` and `SendMessage` and their inbound controls; that is the "never take over, never redirect" line (DESIGN section on running sessions).
3. What klatch needs from native peers is already met: wake of an idle session by an own-child watcher (section 39, measured interactively and headless), content-free pointers, and posting from outside to an unmodified session (section 14.2) as an undocumented Tier 0 nudge.
4. A forged peer that carries klatch text would arrive "unverified" at best; the hold rule for bypassing sessions would delay it. Zero trust says klatch should not rely on the receiving harness's own message path for content.
5. The cost of a third failure is spent time and a version-specific hack; the gain is nil next to the hook path.

Revisit if: Claude Code documents a way to register a non-session peer or a plugin API for the inbox; or the hook watcher proves insufficient in an interactive case that a peer would fix (none found; the last untested case was a full idle hour, #357).

### 49.6 What this means for klatch

- The ancestry-and-registry identity is in the same evidence class as agent-loom; Cotal's is stronger because a broker authenticates. A klatch daemon (epic #152) gets the same strength from its owned socket and peer pid (section 38).
- Cotal and Codex both warn that unattended wake and unattended hooks can silently not work; klatch should say so at the point of use (doctor and init), not in documentation.
- Tickets filed from this section: #544 (Codex untrusted hooks skip silently; doctor must detect), #545 (probe for a real `codex queue`), #546 (per-message delivery receipts), #547 (age reminders for long-held claims).

### 49.7 Codex hook trust keys, hashes and `codex queue --help`, checked against Codex itself (2026-11-17, Codex 0.147.0 installed and 0.169.0 from npm, macOS, ticket #590)

Method: `codex app-server` (stdio JSON-RPC, no login) started with a temp `CODEX_HOME`, an empty environment and a temp project directory; `initialize`, `initialized`, then `hooks/list` with the project as `cwds`. It returns each hook with its `key`, `currentHash`, `trustStatus` (`untrusted`, `trusted` or `modified`), `source`, `sourcePath`, `timeoutSec`, `async`, and warnings. Scripts were in the session scratchpad, not in the repository. **klatch itself never runs `codex app-server`**: the process wrote SQLite state files and a plugin sync directory under the Codex home and ran `git`, which breaks doctor's read only and no network rules. It was used only to learn the facts below.

- **Key indices, verified.** The key is `<file>:<event_snake>:<n>:<m>`: `n` is the index of the matcher group in the event's array, `m` the index of the handler in that group's `hooks` (a file with two groups and a second group of two handlers gave `session_start:0:0`, `:1:0`, `:1:1`). The same on 0.147.0 and 0.169.0.
- **Inline hooks, verified.** Hooks written in `config.toml` as `[[hooks.Stop]]` with `[[hooks.Stop.hooks]]` are listed with `source` `user` and the key `<CODEX_HOME>/config.toml:stop:0:0`. A file that holds both a `hooks.json` and inline hooks gets the warning "loading hooks from both ...; prefer a single representation for this layer".
- **The hash, verified.** `currentHash` is `sha256:` plus the SHA-256 of the compact JSON, keys sorted, of `{"event_name": <snake>, "hooks": [{"async": bool, "command": ..., "statusMessage": only when set, "timeout": seconds, "type": "command"}], "matcher": only when set}`. `timeout` defaults to 600, `async` to false, unknown fields do not count, a matcher of `""` or `"*"` counts as set, and a handler field spelled `timeoutSec` in the file is ignored (the file field is `timeout`). Five real hashes were reproduced exactly by this rule (the tests carry two of them, the SessionStart hook with the matcher `startup` and the inline Stop hook; the SessionStart hook with no matcher in the tests is computed by the same rule, not read from Codex). A record whose `trusted_hash` is not the current hash lists as `modified`; no record lists as `untrusted`. The same hash on both versions.
- **Async hooks are skipped.** Codex prints "skipping async hook ...: async hooks are not supported yet" and does not list them. A klatch wake hook (`asyncRewake`) therefore cannot run in Codex on either version, and a Codex hook installer must not add one.
- **Project hooks.** A `.codex/hooks.json` in the project was not listed (the project layer was not trusted). Where Codex keys trust for a project hook was not observed, so klatch reports a klatch hook there as unchecked.
- **`codex queue --help` on a binary that has the command (0.169.0), verified.** It prints `Queue a message for an existing session`, then `Usage: codex queue [OPTIONS] --thread <THREAD> --message <TEXT>`; options include `--thread <THREAD>` (session UUID or exact name), `--message <TEXT>`, `--resume`, `--remote <ADDR>`, `--remote-auth-token-env`. The usage line match in `codex.rs` is therefore right; 0.147.0 still has no `queue`.
- **What changed in klatch.** Doctor and init now judge each klatch hook by the verified key, compare the recorded hash with the recomputed one (a changed hook reads as not trusted), read inline `config.toml` hooks, report an async klatch hook as skipped, and report a klatch hook in the project `.codex/hooks.json` as unchecked. The hash is only compared, never written. If a future Codex changes the hash rule, doctor would call a trusted hook changed; the fix text says to review it in `/hooks`, which clears it.

### 49.8 What `klatch init --codex` writes, listed by Codex itself (2026-11-17, Codex 0.147.0, macOS, ticket #782)

Method: the same `codex app-server` `hooks/list` call as 49.7, in a temp `HOME` and `CODEX_HOME` (no login, no model, empty environment, temp project), after `klatch init --codex --yes` had written the temp `hooks.json`. The live `~/.codex` was never read or written.

- **Listed, verified.** Codex 0.147.0 lists exactly the three hooks klatch writes (`session_start:0:0`, `user_prompt_submit:0:0`, `post_tool_use:0:0`), `source` `user`, `trustStatus` `untrusted`, with no warning and no error. So the file shape (the Claude settings shape, `"hooks": {Event: [{"hooks": [{"type": "command", "command": ...}]}]}`) and the command `<bin> --codex hook --event <Event>` are accepted as written; no `timeout` or `async` field is needed.
- **Not installed on purpose.** The wake hook (Codex skips async hooks), the claim hook (PreToolUse) and SessionEnd (no run of either was seen in Codex, so klatch does not claim them).
- **Needs a human and a login, not verified.** Trusting the hooks in `/hooks`, and whether `codex exec` runs them once trusted (needs a Codex login for a model turn). klatch never writes the trust record.

## 50. Headless Claude Code sessions as verified team members (2026-11-15, Claude Code 2.1.325, macOS, ticket #582)

Question: can the human run the next wave of work as separate `claude -p` sessions, each a distinct verified klatch member with its own bundle, worktree and hooks, instead of Agent-tool subagents that share the parent's name?

### 50.1 What was built

`proto/scripts/team` (DESIGN 62): `start TASKS_FILE`, `status`, `logs NAME`, `stop`, `smoke`. Tested by `proto/scripts/test-team.sh` against a fake klatch and a fake claude.

### 50.2 What `claude -p` supports (checked with `claude --help`)

`--settings FILE`, `--setting-sources`, `--append-system-prompt`, `--output-format stream-json` (needs `--verbose`), `--max-budget-usd`, `--permission-mode`, `--model`, `--no-session-persistence`. There is no `--max-turns` in this version, so the launcher uses a wall-clock watchdog instead (`TEAM_MAX_SECONDS`) plus the dollar cap.

### 50.3 Real run (Haiku, the settings file and runner the script generates, a FAKE klatch that logs each call)

Method: one task, "run `echo hi`, then `echo bye`, then finish; say what any hook told you". The fake `hook --event SessionStart` printed a join line; the fake `hook --event PostToolUse` printed `{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"klatch mail: ..."}}` once.

| Observation | Result |
|---|---|
| Startup | the SessionStart hook ran in the same second the launcher started; the first model turn followed within the second |
| Hooks fired | SessionStart, UserPromptSubmit (once: one prompt), PostToolUse after every tool call, Stop, SessionEnd, in that order; each ran with `KLATCH_IDENTITY_FILE` set to the bundle and no name |
| Mail between turns | the PostToolUse context reached the model after the two Bash calls and it acted on it (ran the extra command). Mail is delivered mid-turn at tool boundaries, not between prompts, because there is one prompt |
| Cost and time | 4 turns, 9.2 s, about $0.003 (Haiku) |
| Exit | 0; the runner wrote the exit file and revoked the credential after the SessionEnd hook |

The model treated mail as an instruction from the user ("the harness treats hook output as user feedback"), so the brief must keep saying mail is data, and a hostile message in a room is a prompt injection path for every worker (the labelled digest of a sender being verified helps, it does not stop this).

### 50.4 asyncRewake in `-p` (question of RESEARCH-NOTES 47)

With `TEAM_WAKE=1` (a SessionStart and a Stop watcher that sleeps 8 s then exits 2 with a pointer on stderr), a task that answers in one turn: the session answered at 2 s and the process stayed alive until the watchers ended (SessionEnd at 10 s). The pointer text never reached the model (1 turn, the marker word absent from the log). So a headless session is held open by its async hooks but is not woken by them; with the real 540 s watcher a finished worker would sit for up to nine minutes. The wake watcher stays off for workers, and a worker waits with `klatch wait` inside its turn.

### 50.5 NOT run, and why

The real smoke through klatch (`worker issue`, a verified post, `read` showing a sender without `(unverified)`) was not run by this session: `klatch worker issue` refuses inside a coding-agent session, by design, and nothing here detaches from the process tree to get around it. The human runs `proto/scripts/team smoke` in a terminal (a throwaway home and key directory; one Haiku worker joins, posts, reads, finishes) and `proto/scripts/team start tasks.txt` for the wave. decision 14 of the 2026-11-14 decisions (DESIGN section 32) already records a headless session verified through its bundle (`w-live`); what is new here is the launcher around it.

### 50.6 Failure modes the script handles

A refused issue starts nothing for that task (no worktree, no claude), prints claude's reason and the others go on, exit 1; a bad tasks file is refused whole with exit 2 before anything is issued; a worker already running is not started twice; a finished worker is started again on its existing branch and worktree; `stop` kills the runner and its claude and revokes; the watchdog kills a worker past its wall clock and the runner revokes.

### 50.7 Tickets

#591 (a finished headless turn cannot receive mail), #592 (real smoke through klatch, needs-human), #593 (no turn cap, team budget or relaunch), #594 (room mail as a prompt injection path in bypassPermissions, needs-human).

### 50.8 The real smoke run through klatch (2026-11-15, ticket #592)

Run by the human in their own terminal: `TEAM_KLATCH=<this checkout's debug build, 0.1.0-dev.2, schema 28> proto/scripts/team smoke`. One Haiku session, throwaway home and repo. Result: done, exit 0, 4 turns, 9.2 s, $0.0035. The worker joined the room, read it, posted a hello line and `status: done`. The launcher's view of the room showed both worker posts **without** the `(unverified)` tag, so a headless session started by the launcher acts as a **verified** member through its worker credential. Not checked in this run: that its credential was revoked afterwards (the log shows the session ended; the revoke was not read back), and mail delivery between turns through a real klatch (only the fake-klatch run in 50.3 showed that).

## 51. A Quint model of the claim and lease protocol, checked with Apalache and replayed against the gateway (2026-11-15, Quint 0.33.1, Apalache 0.62.4, Node 22.15.1, Java 22, macOS, ticket #405, decided in #402)

Question: can the claim and lease protocol (overlap, exclusive and shared scopes, expiry, renewal to a claim's own ttl, liveness leases, parking, the 24 hour cap, cycles of waits) be written as a small model with named invariants that a model checker explores to a bound, and can that model be tied to the real gateway so the two cannot drift apart unnoticed?

### 51.1 What was built (DESIGN 76)

`proto/klatch/models/claims.qnt` (about 400 lines of Quint), `proto/scripts/quint-check` (the runner), `proto/klatch/tests/model_replay.rs` (the link), and 19 committed traces under `proto/klatch/models/traces` (250 KB). Tools are installed outside the repository: `npm i --prefix ~/.local/share/klatch-quint @informalsystems/quint`; Quint fetched Apalache itself into `~/.quint` on the first `quint verify`. Node and Java were already present. The model has seven actions (tick, claim, release, activity, post, park, settle) over three members, three paths (`a`, `a/b`, `c`, so one overlap), a shared and two private scope roles, and the real constants in minutes (cap 1440, wait window 2, liveness extension 10, grace 5, unpark cut back 10, park maximum 10080).

### 51.2 Result: model checked up to 8 steps and 3 members

`proto/scripts/quint-check` (random runs, then Apalache per invariant, one at a time so a failure names itself). No violation of any of the 12 invariants; run times on this Mac (Darwin 23.3.0), one Apalache process each:

| invariant | what it says | steps | time |
|---|---|---|---|
| `NoOverlappingLiveExclusive` | no two live claims by different members overlap in colliding scopes | 8 | 189 s |
| `LeaseWithinCap` | a live claim of a member that is not parked (no park row, or its return time passed) ends within 24 h of now (reworded from "never parked" by #646; the 8 step, 331 s figure is for the earlier wording; the reworded one is checked to 6 steps, 228 s, see 51.7) | 8 | 331 s |
| `LeaseWithinParkMax` | no live claim ends more than 7 d out | 8 | 231 s |
| `StuckOwnerHoldsNothing` | after a stuck verdict the owner holds nothing | 8 | 34 s |
| `ReleaseOnlyOwn` | a release removes only the releaser's own claim | 8 | 35 s |
| `ExpiredNeverBlocks` | a refusal is by a live claim, never an expired row | 8 | 34 s |
| `OnlyLivenessRevives` | a lapsed row is live again only by an alive verdict | 8 | 876 s |
| `RenewNeverRevives` | renewal, a post, a park never revive a lapsed claim | 8 | 266 s |
| `RenewNeverShortens` | renewal never shortens a lease | 8 | 180 s |
| `ExpiryShrinksOnlyByLeaseRules` | a lease shrinks only by a liveness rule or the unpark cut back | 8 | 1307 s |
| `DeadlockFlagExact` | a refusal says deadlock exactly when it closes a cycle of waits | 7 | 506 s |
| `RefusalTellsOnlyTheRequester` | a refusal changes no other member's wait | 7 | 12 s |

The two deadlock invariants run on the smaller step relation `stepCore` (claim, release, tick; ttl 10 or over the cap, steps of 1 or 3 minutes) and to 7 steps, because at 8 steps `DeadlockFlagExact` had run over half an hour without finishing. The other ten run on all seven actions. The first ten rows are from one run, the last two from a second run made after the last edit of the model, which changed only the second of them (the model's actions did not change between the runs). Every run also does 3000 random runs of 30 steps per invariant (`quint run`), all passing.

Wording: this is **model checked up to 8 steps and 3 members**, not proven. The model leaves out `ask:` claims and their short leases, anchors, release notes, claim ages, rooms and membership changes, and treats the busy-for-the-cap verdict as stuck. Apalache explores every behaviour up to the bound with the values the model offers (four ttls, six clock steps, five park spans, four verdicts); a counterexample needing more steps, another value, a fourth member, or one of the left-out pieces is not excluded.

### 51.3 Each invariant can fail: bugs planted in the model

`proto/scripts/quint-check mutations` rewrites one `BUG` line of a temporary copy and runs the invariant that should catch it. All 15 were caught within 8 steps (two more were added for #646, see 51.7): `equalOnly` and `sharedIgnored` (`NoOverlappingLiveExclusive`), `capOff` and `extendOverCap` (`LeaseWithinCap`), `parkUncapped` (`LeaseWithinParkMax`), `stuckKeeps` (`StuckOwnerHoldsNothing`), `releaseAny` (`ReleaseOnlyOwn`), `expiredBlocks` (`ExpiredNeverBlocks`), `renewRevives` (`RenewNeverRevives`), `parkRevives` (`OnlyLivenessRevives`), `renewShortens` (`RenewNeverShortens`), `parkShortens` (`ExpiryShrinksOnlyByLeaseRules`), `deadlockNever` and `windowIgnored` (`DeadlockFlagExact`), `tellAll` (`RefusalTellsOnlyTheRequester`). One lesson: the first run did **not** catch `sharedIgnored`, because the invariant used the same `collide` function the bug had broken, so it agreed with the bug. The invariants now use a clean copy of each relation and only the actions use the buggy one. A planted bug is how that vacuous check was found.

### 51.4 Each invariant can fail: bugs planted in the real code

Each of these was applied to the real source one at a time, `cargo test -p klatch --test model_replay` run, and the change reverted (the tree was clean after): overlap by equality only (`domain/text.rs`), renewal that revives a lapsed claim and renewal that also shortens (`store/sqlite.rs`), `closes_cycle` always false, no 24 hour cap in `Gateway::claim`, `Verdict::Stuck` mapped to leave, a park not cut to seven days (`domain/parked.rs`), shared scope that does not collide (`domain/scope.rs`), release that ignores the owner, waits that never expire, an expired claim that still blocks, a liveness extension of 25 hours and a grace period of 10 minutes (`lease.rs`), and the unpark cut back applied to a member that is not parked. 14 of 14 failed the replay with a message naming the trace, step, and the two outcomes. Two were **not** caught by the first six random traces and six witnesses (waits that never expire; the cut back applied to everyone); adding two witnesses (`W_staleWait`, `W_postKeeps`, a refusal after a wait aged out and a post by a member holding a long claim) made both fail. So the replay is only as good as its traces: when the model gains an action or a rule, add a witness for it.

### 51.5 What the model found: four design gaps, ticketed

Kept as invariants that were expected to fail (`quint-check findings`, each reported "violated, as recorded"); all four are now fixed or decided and hold, and the findings list is empty. `StrictLeaseCap` (a park stretches a claim to seven days, past D5's 24 hours: #646, 2 step trace) was decided on 2026-11-16: a park is the one exception to the cap, so the invariant became `LeaseWithinCap` ("every claim of a member that is not parked ends within 24 h"; parked means the park row's return time is still ahead) and `LeaseWithinParkMax` (nothing ends more than 7 days out, parked or not) bounds the exception; the planted bugs `postKeepsStretch` (a post does not cut a park's stretch back, which the old "never parked" wording could not see and the new one catches) and `parkOverMax` (the park maximum one minute too long) were added, see 51.7. The other three were: `ParkedLeaseEndsAtReturn` (a shorter re-park leaves the longer stretch: #647, 3 steps), `EveryCycleHasATold` (a cycle completed by a claim being taken, not by a refusal, is told only at the next retry: #648, 5 steps), and `ActivityRenewsToOwnTtl` (the unpark cut back runs after the post's renewal and goes below the claim's own ttl until the next action: #649, 3 steps). The park exception is stated by two holding invariants: `LeaseWithinCap` for members that are not parked and `LeaseWithinParkMax` for everyone. No violation of the other invariants was found: no two live overlapping claims in colliding scopes, no lease past the cap or the park maximum, no renewal that revives or shortens, a refusal says deadlock exactly when it closes a cycle.

Two properties did not survive their first wording, and the second was found only by Apalache. "A cycle of waits makes exactly one party yield", stated over the told flag of every live wait, failed in a 3000 by 30 step random run: two told waits on one cycle (a member told earlier that did not yield, then a second closure through it). I narrowed it to "the attempt that closes a cycle that did not exist before is the only told wait on that cycle"; 5000 random runs of 30 steps passed it, but Apalache found a 6 step counterexample in 38 s: m3 is told, then re-takes its own claim in another scope (which breaks the cycle without yielding), then m2 closes a new cycle through m3's still stored told wait. The wait record keeps `told` from an earlier step, so any statement over stored flags is about history, not about the step. What the protocol actually promises, and what is now checked as `RefusalTellsOnlyTheRequester`, is step local: a refusal changes no wait but the requester's, and (`DeadlockFlagExact`) the requester is told exactly when its attempt closes a cycle. Together: at the moment a cycle closes, one party is told. A retry while a cycle persists is told again by design, and a member that was told and does not yield stays on the cycle; neither is a second yielder created by the protocol.

### 51.6 Practical notes

- Apalache cost is dominated by the cycle test. A closure built as a set of pairs (`fold` over `filter`) did not finish a 5 step check in 3 minutes; the same question as quantifiers over the three members (`walk3`) took 36 s for 6 steps. The deadlock invariants also run on a smaller step relation (`stepCore`: claim, release, tick), 7 s to find a deadlock trace instead of being intractable. The runner says so.
- Shortest traces per event come from a witness trick: state "this never happens" as an invariant (`W_deadlock`, `W_revive`, ...), let Apalache violate it, and keep the counterexample as a replayable trace. One witness (`W_staleWait`) is cheaper to find by random search (`quint run`, 1.4 s) than by Apalache.
- `quint run --out-itf` and `quint verify --out-itf` write ITF JSON (`{"#bigint": "12"}` for numbers, `{"#set": [...]}` for sets); `serde_json` reads it with no new dependency. The replay maps one model minute to 60 seconds and compares the outcome of each claim or release and the live claims.
- The model's replay needed one fix in the test, not the code: the gateway refuses a post identical to the previous one within moments, so the replay numbers each post.
- Quint rejects `val` operators with parameters and a few other things Apalache users meet (`next` is a built-in name; map types are written `str -> int`).

### 51.7 The park exception to the 24 hour cap (#646, decided 2026-11-16)

The human decided that a park is the one exception to D5's 24 hour claim cap: an unparked member's claims stay within 24 hours; a park holds them at most 7 days (`PARKED_MAX_SECS`), then they lapse as DESIGN item 46 describes. No gateway change was needed (the code already did this); DESIGN D5 and items 25, 46, 52 and 76 now say it.

The model change: the finding `StrictLeaseCap` ("no live claim ends more than 24 h out, parked or not", a 2 step trace) became `LeaseWithinCap`, now "a live claim of a member whose park row does not run past now ends within 24 h". That is stronger than the old `LeaseWithinCap`, which exempted every member that had *ever* parked (the ghost `everParked`, now read by nothing and kept only so the committed traces stay valid). `LeaseWithinParkMax` is unchanged and bounds the exception at 7 days. `quint-check findings` now lists none.

Red first: the new check is only worth having if it sees something the old one could not. The planted bug `postKeepsStretch` (a post that ends a park does not cut the stretched claims back, so an unparked member keeps a 7 day claim) was added first. Against the OLD `LeaseWithinCap` it was **not caught** (Apalache, 8 steps, 3 members: the member has parked, so it was exempt). After the reword it is caught. My first wording of the reword had the comparison backwards (`parkedUntil <= now` for "parked") and failed at once in a random run and in Apalache; it was corrected before any result was recorded.

Planted bugs (the list is now 17): `postKeepsStretch` (`LeaseWithinCap`) and `parkOverMax` (the park maximum one minute too long, 10081 minutes; `LeaseWithinParkMax`) are new; with them, `capOff`, `extendOverCap` (`LeaseWithinCap`) and `parkUncapped` (`LeaseWithinParkMax`) were re-run on the reworded invariants and are caught, all at a bound of 6 steps and 3 members. The other 12 mutants were not re-run: their invariants and the model's actions did not change (only the two new mutant switches were added).

Bound, honestly: the reworded `LeaseWithinCap` and `LeaseWithinParkMax` were model checked with Apalache up to **6 steps** and 3 members (228 s and 118 s, Apalache 0.62.4) plus 3000 random runs of 30 steps each. An 8 step run of `LeaseWithinCap` did not finish in 48 minutes and was stopped, so 8 steps was not reached. Not re-run after this change: the other 13 invariants. The replay test `model_replay` and its 19 traces are unchanged and pass.

## 52. Kani bounded proofs of the pure security functions (2026-11-15, Kani 0.68.0, CBMC 6.11.0, nightly-2026-08-21, Apple M3 Pro, macOS Darwin 23.3.0, ticket #404, decided in #402)

Question: can a model checker give a bounded proof, run by hand and never on a pull request, for the pure functions
that carry the security decisions? Answer: yes for every area asked for except the recovery code (ticket #656), at small bounds, and only after
restructuring four functions so the solver can finish. Every result below is a
bounded proof, "no counterexample up to N", not a proof for all inputs.

### 52.1 What was built

`proto/klatch/src/kani_proofs/` (compiled only under `cfg(kani)`, declared in `lib.rs`, with `unexpected_cfgs`
told about `kani` in `Cargo.toml`), `proto/scripts/kani` (runs every harness one at a time with a time limit, prints
pass or FAIL and the seconds; a harness that times out is a FAIL), and a section in CONTRIBUTING.md. Nothing in CI,
the gate or any build runs them. Install: `cargo install --locked kani-verifier` (0.68.0) and `cargo kani setup`
(downloads CBMC and a pinned nightly).

Bolero was not used. I did not install `cargo-bolero` or add the `bolero` crate to a build, so I did not try its Kani
engine end to end; the reasons are that it adds a dependency (and a licence line for THIRD-PARTY-LICENSES.txt) to
reach the same `#[kani::proof]` harnesses, and that its generators for `String` and `Vec` build the heap values this
section shows the solver cannot finish. If someone wants a bolero harness style later, the bounds below are the
starting point. The crate already uses Hegel for property tests.

### 52.2 Results (final run, one harness at a time, 2026-11-15)

All 21 harnesses pass. Total run time about 30 minutes on one core at a time; the three clean_path harnesses are
two thirds of it. Times are wall clock for `cargo kani --harness NAME` including the incremental build.

| Harness | Function | Bound | Time |
|---|---|---|---|
| clean_path_is_normal | clean_bytes (the logic of clean_path) | path and cwd up to 3 bytes each from `/ . a`, 8 byte output; result starts with `/`, has no `//`, no trailing `/` (except the root), no `.` or `..` segment | 91 s |
| clean_path_is_idempotent | clean_bytes | same bounds; cleaning a cleaned path changes nothing | 236 s |
| clean_path_dots_never_escape_the_root | clean_bytes | 2 or 3 `..` segments as a relative path, cwd up to 4 bytes from `/ . a`; the result is the root | 676 s |
| clean_path_dot_dot_removes_exactly_one_segment | clean_bytes | one byte names a or b; `/x/y/..` is `/x`, `y/..` from `/x` is `/x` | 12 s |
| overlaps_is_symmetric_and_reflexive | overlaps | each path up to 3 bytes from `/ a b` | 194 s |
| overlaps_is_by_component_not_by_text_prefix | overlaps | directory and child names of one byte (a or b): `/a` and `/a/b` overlap, `/a` and `/ab` do not, a trailing slash changes nothing | 71 s |
| valid_name_accepts_only_the_safe_alphabet | valid_name | up to 4 ASCII bytes; accepted implies non-empty and only `A-Za-z0-9 - _ .`, and the converse | 77 s |
| valid_kind_accepts_only_the_safe_alphabet | valid_kind | same | 45 s |
| valid_scope_accepts_only_the_safe_alphabet_and_never_the_shared_marker | valid_scope | same; accepted implies not `*` | 24 s |
| non_ascii_is_refused | the three above | four concrete non-ASCII strings | 47 s |
| name_length_limit_holds_at_the_boundary, kind_..., scope_... | valid_name, valid_kind, valid_scope | 40, 40 and 80 bytes accepted, one more refused; concrete text | 7, 6, 10 s |
| scopes_collide_is_symmetric_and_shared_collides_with_all | scopes_collide | up to 3 bytes from `a b *` | 43 s |
| dominance_is_reflexive_total_and_antisymmetric_and_fails_closed | dominates_in (what LabelChain::dominates calls) | chain of 2 to 4 names (prefix of public, internal, private, secret); labels those four or an outside one | 142 s |
| dominance_is_transitive | dominates_in | same, three labels | 24 s |
| loopback_is_exactly_127_slash_8_and_the_ipv6_forms | is_loopback | none: every IPv4 and IPv6 address | 7 s |
| check_bind_allows_plaintext_only_on_loopback_with_a_token | check_bind | none on the address or port; all three Security values and both token values | 5 s |
| a_blob_is_protected_only_with_the_full_header_and_the_magic | looks_protected, reveal | blobs of 0 to 30 bytes, any content (the header is 28); short input is refused as Malformed | 35 s |
| an_unknown_version_or_algorithm_is_refused_before_decryption | reveal | 30 byte blob with the magic and any version and algorithm other than 1 and 1; refused by name before the cipher runs | 12 s |
| sealed_text_is_recognised_by_its_prefix | is_sealed_text | twelve concrete texts, no symbolic input (an example test run under the checker, not a search) | 7 s |

Two harnesses have no bound on their main input (the address functions); for them "bounded proof" means only that
the Security and token inputs are enumerated.

### 52.3 What is not covered

- The recovery code (keys/recovery.rs): no harness. A harness for `restore` over 6 characters timed out at 300 s and
  at 600 s, and over 2 characters (a real code is 63) at 500 s, because it filters into a `String`, decodes base32
  into a `Vec` and hashes with SHA-256. Ticket #656.
- clean_path's wrapper (a `vec!` and `String::from_utf8_lossy`) and paths over 3 bytes; ticket #659. Guarded by a
  unit test that compares the new form with the old split-and-join form on every string of up to 5 characters.
- valid_name, valid_kind, valid_scope on strings of 5 to 39 characters; ticket #657.
- LabelChain::new, parse, min, default_cap and the wrapper methods; ticket #658.
- overlaps and scopes_collide on longer text; ticket #660.
- Whether a well-formed sealed value opens (the AEAD), whether any transport calls check_bind (a caller property),
  name resolution, symlinks.
- is_sealed_text has only concrete examples: validating symbolic bytes as UTF-8 (sixteen bytes, one byte at a symbolic
  position, even forty-eight concrete validations) did not finish in 600 s, and `from_utf8_unchecked` is refused by
  the unsafe-code check in tests/hygiene.rs. The text form is one `starts_with` on a constant.

### 52.4 What the solver could not do, and what that cost the code

Anything that grows a heap value with a symbolic size is out of reach at useful bounds. Calling `format!`, `split`,
`join`, `to_string`, `String::push` or `Vec::push` under a symbolic length gave programs of 300,000 to 640,000 steps
and 7 to 12 million variables, and timeouts at 300 to 600 s even for one symbolic character. The same logic on a
fixed array finishes in seconds to minutes. So four functions changed shape, with the same results:

- `clean_path` now allocates once and calls `clean_bytes`, which works on byte slices and an output buffer.
- `overlaps` trims with a byte loop instead of `norm` (which allocates) and `format!`.
- `valid_scope` checks bytes instead of `chars().count()` and `contains`.
- `LabelChain::dominates` and `rank` delegate to `dominates_in` and `rank_in` on a slice of names.

Each is pinned: the old forms of clean_path and overlaps live in the tests as reference functions and are compared on
every short input (clean_path 5 characters over `/ . a` against six working directories; overlaps 5 characters over
`/ a b`), and the existing unit tests for all four pass unchanged. A symbolic index into an array of `&str` was also
costly (the first clean_path harness did not finish in 600 s even on three concrete inputs), as was `assert_eq!` on
strings (its formatting code); the harnesses use byte arrays, `if` chains and `assert!(a == b)` for that reason.

### 52.5 Each harness can fail: planted bugs

For each harness I planted a bug in the real function, ran the harness, and reverted. CAUGHT means `VERIFICATION:-
FAILED`.

| Planted in | Bug | Harness | Result |
|---|---|---|---|
| clean_bytes | `..` ignored | clean_path_dot_dot_removes_exactly_one_segment | caught, 6 s |
| clean_bytes | `..` goes to the root | clean_path_dot_dot_removes_exactly_one_segment | caught, 5 s |
| clean_bytes | `.` segment kept | clean_path_is_normal | caught, 79 s |
| clean_bytes | an absolute path joined to the cwd | clean_path_is_idempotent | caught, 234 s |
| clean_bytes | `..` ignored | clean_path_dots_never_escape_the_root | caught, 52 s |
| overlaps | match by text prefix, not component | overlaps_is_by_component_not_by_text_prefix | caught, 56 s |
| overlaps | one direction only | overlaps_is_symmetric_and_reflexive | caught, 170 s |
| valid_name | space allowed | valid_name_accepts_only_the_safe_alphabet | caught, 67 s |
| valid_name | any Unicode letter allowed | non_ascii_is_refused | caught, 198 s |
| valid_kind | colon allowed | valid_kind_accepts_only_the_safe_alphabet | caught, 43 s |
| valid_name, valid_kind, valid_scope | limit one byte too high | the three length harnesses | caught, 10, 10 and 15 s |
| valid_scope | star allowed in the alphabet | valid_scope_accepts_only_the_safe_alphabet_and_never_the_shared_marker | caught, 23 s |
| scopes_collide | shared marker ignored on the right | scopes_collide_is_symmetric_and_shared_collides_with_all | caught, 38 s |
| dominates_in | unknown label dominates | dominance_is_reflexive_total_and_antisymmetric_and_fails_closed | caught, 128 s |
| dominates_in | a label dominates the one above it | dominance_is_transitive | caught, 27 s |
| is_loopback | mapped IPv6 loopback not loopback | loopback_is_exactly_127_slash_8_and_the_ipv6_forms | caught, 5 s |
| check_bind | plaintext needs no token | check_bind_allows_plaintext_only_on_loopback_with_a_token | caught, 5 s |
| looks_protected | header one byte short accepted | a_blob_is_protected_only_with_the_full_header_and_the_magic | caught, 31 s |
| reveal | unknown algorithm falls through | an_unknown_version_or_algorithm_is_refused_before_decryption | caught, 12 s |
| is_sealed_text | prefix one character short | sealed_text_is_recognised_by_its_prefix | caught, 7 s |

### 52.6 What the planted bugs taught

- The normal-form harness did not catch a `..` that is silently ignored: the result is still in normal form. A
  semantic harness (`..` removes exactly one segment) was added because of that; the first set of plants is what
  showed the gap.
- The dot-climbing harness first built an absolute path (`/../..`), so the working directory was ignored and the
  harness would have passed with any handling of `..`. The planted bug passed it; the path was changed to relative and
  the plant then fails it. A proof whose input makes the interesting part vacuous is the failure mode to watch for.
- A plant that removes `scope != SHARED_SCOPE` from valid_scope is not caught, and cannot be: `*` is not in the
  alphabet, so the line is unreachable (ticket #664). A plant that widens the alphabet to include `*` is caught.
- A plant of `text.contains(prefix)` in is_sealed_text ran past 900 s, so it was replaced by a cheaper plant; a harness
  that is slow on the buggy code is slow on the good code too.
- A plant that keeps `.` segments passes the idempotence harness (the buggy function is stable on its own output); the
  normal-form harness catches it. Each property needs its own plant.

### 52.7 Tickets

#656 the recovery code has no proof; #657 valid_* beyond 4 characters; #658 LabelChain::new and parse; #659 clean_path
wrapper and longer paths; #660 overlaps and scopes_collide at 3 characters; #664 the unreachable shared-marker check.

## 53. Shuttle tests of the std-thread code (2026-11-15, Shuttle 0.9.10, shuttle-std 0.1.5, Apple M3 Pro, macOS Darwin 23.3.0, ticket #406, decided in #402)

Question: which std-thread code in the klatch crate shares state between threads, and can Shuttle's randomized
schedules find an interleaving bug in it? Answer: four small pieces qualify, all four run under Shuttle, and each test
failed when a bug was planted. A pass is "no failing schedule among those tried", not a proof.

### 53.1 What shares state between threads

A search of `proto/klatch/src` for Mutex, atomics, channels, Condvar and thread spawns found: the message id counter
(`store/ids.rs`, one `AtomicU32` and a `OnceLock`); the session input reader (`session/mod.rs`, a thread feeding a
channel that the session loop drains with `collect`); the client's reader thread (`client.rs`); and three copies of
"run a probe on a thread and wait with `recv_timeout`" (doctor.rs, keys/macos.rs, identity.rs `stdin_json`). The rest
is test code (session tests, custody and sqlite concurrency tests) or single-threaded by type: `Gateway` and
`Subscription` hold `Rc` and `RefCell`, so a gateway cannot be shared between threads at all. The wait and wake commands
poll with sleeps in separate processes; there is no in-process notifier.

### 53.2 What changed shape

- `src/sync.rs` re-exports `std` or Shuttle by `cfg(shuttle)`; the code above imports from it.
- The three copies of `within` became one `sync::within` (the Keychain and `stdin_json` call it; behaviour is the same).
- `Client::connect`'s reader closure became `client::pump_frames(reader, tx)`; `read_in_background` and `collect` became
  `pub(crate)`; `store::ids` became `pub(crate)`.
- `COUNTER` in ids.rs has two definitions: a plain static, and under Shuttle a `shuttle::lazy_static!` (a plain static
  would carry its count from one explored run into the next).
- `shuttle` is a `[target.'cfg(shuttle)'.dependencies]` entry, so it is in Cargo.lock but a normal build never compiles
  it. The Shuttle build goes to `proto/target/shuttle`.

### 53.3 Results (fixed seed 406)

| Test | Schedules | Time |
|---|---|---|
| ids stay unique, 3 threads x 2 ids in one millisecond | 2000 | under 1 s together with the rest |
| session reader: 4 frames once, in order, then closed | 2000 | same |
| client reader: 3 frames, then the end marker last | 2000 | same |
| within: two probes at once each get their own answer | 2000 | same |

All pass at 2000 and at 200000 schedules each (`proto/scripts/shuttle long`, 55 s total).

### 53.4 Planted bugs (`proto/scripts/shuttle mutations`)

Each plant was put in the real code, the test failed with a Shuttle schedule, and the code was reverted.

| Plant | Caught by |
|---|---|
| id counter read then written instead of one `fetch_add` (a lost update, duplicate ids) | the ids test |
| `collect` treats an empty channel as a closed input (a late frame is dropped) | the session reader test |
| `pump_frames` sends the end marker before the frames | the client reader test |
| `within` uses `try_recv` instead of waiting (answers None when the probe is slow) | the within test |

Honest notes: the tests were written before the code they exercise existed, so they were red only as compile errors;
the first runs were green. The planted bugs are what show they can fail. The client reader plant fails on any schedule
(an ordering bug, not a race), so it shows the test checks order, not that Shuttle was needed.

### 53.5 What is NOT covered

- Time. Shuttle's `recv_timeout` never times out (#666): within's time limit, heartbeats, poll intervals, busy retries.
- The store, SQLite, SQLCipher, `flock` and OS locks, the Keychain, and everything across processes (#667); the fault
  tester (#625) and the Quint model (#405) are the evidence for those.
- The gateway: it is not shared between threads by construction.
- Ordinary tests cannot run in a Shuttle build (#668), so only `shuttle_tests::` runs there.
- Shuttle explores random schedules (here the random scheduler with one fixed seed), so a rare interleaving can be
  missed. Loom (exhaustive within bounds) was not tried: it needs wrapped primitives everywhere and the four pieces
  are small enough that random schedules at 200000 each are a reasonable first measure.

### 53.6 Tickets

#666 time is not modelled; #667 what Shuttle cannot reach; #668 ordinary tests do not run in a Shuttle build.

### 53.7 Update, 2026-11-18: a Shuttle-visible time limit, and listeners that go away (#666)

`sync::within` under `--cfg shuttle` now races the probe against a timer thread (see DESIGN item 102), so Shuttle explores
the limit passing at any step. Eight tests run (the four from section 53.3, one of them, the `within` hand-off test, reworded because
the answer can now lose the race, and four new ones), 2000 schedules each, about a second
together once built; the gate's 200-schedule smoke is unchanged in cost. Results are "no failing schedule among those
tried", never a proof.

| Test | What it catches (planted bug, caught by `proto/scripts/shuttle mutations`) |
|---|---|
| `within_gives_up_on_a_probe_that_never_answers_in_every_schedule` | the expiry is never sent: the waiter blocks, Shuttle reports a deadlock |
| `within_answers_the_probes_value_or_none_and_a_late_answer_is_harmless` | the probe thread unwraps its send, so an answer after the limit panics it; also the waiter that drops every answer (the test requires both outcomes among the schedules) |
| `within_hands_each_waiter_its_own_probes_answer_or_none_never_the_others` | (no plant of its own; the old hand-off plant, a waiter that polls once, is the same as the limit passing at once and cannot be told apart) |
| `the_session_reader_ends_quietly_when_the_session_stops_listening_mid_stream` | `spawn_reader` unwraps its send |
| `the_client_reader_ends_quietly_when_the_receiver_is_gone` | `pump_frames` unwraps its send |

Honest notes. The first plant I tried for `within` (the waiter uses `try_recv` and gives up at once) was MISSED: under the
model, giving up before the probe answers is one more legal schedule, so no test can tell it from the limit passing
first. It was replaced by a plant that throws the answer away. The new tests were red before the change for the two
that need the timer (a deadlock and a missing outcome) and green on first run for the two reader tests, whose plants are
what show they can fail. #667 (the store and the gateway) and #668 (ordinary tests in a Shuttle build) were not changed
and carry needs-human questions.

### 52.8d Update, 2026-11-18: one step past the first bounds, and the clean_path wrapper (#659, #660; Kani 0.68.0)

| Harness | Bound | Time |
|---|---|---|
| clean_path_is_normal_up_to_4_bytes | `clean_bytes`, path and cwd up to 4 bytes from `/ . a`, 10 byte output (the first set stopped at 3 bytes, 91 s); starts with `/`, no `//`, no trailing `/` except the root, no `.` or `..` segment | 161 s |
| overlaps_is_symmetric_and_reflexive_up_to_4_bytes | each path up to 4 characters from `/ a b` (was 3, 194 s) | 320 s |
| overlaps_follows_components_two_levels_deep | `/d`, `/d/e`, `/d/e/r` and `/d/er`, names of one byte (a or b); overlaps by component not by text prefix, both directions | 195 s |
| scopes_collide_up_to_5_bytes | up to 5 characters from `a b *` (was 3, 43 s); symmetric, reflexive, and with neither shared they collide exactly when equal | 43 s |

### 52.8b Update, 2026-11-18: LabelChain::new, min and default_cap (#658; Kani 0.68.0)

`LabelChain::new`'s three rules, `min` and `default_cap` were split into `check_chain`, `min_in` and `default_cap_in`
(slices, no allocation; `new` keeps only the message text) and proved at small bounds.

| Harness | Bound | Time |
|---|---|---|
| a_chain_needs_two_or_more_valid_distinct_names_and_the_first_fault_is_named | 0 to 4 names from `a`, `b`, `c` and the invalid `x y`; result and first fault compared with a second formulation in the harness | 610 s |
| min_is_the_lower_known_label_and_none_for_an_unknown_one | chains of 2 to 4 of the default names; labels those or one outside | 42 s |
| the_default_cap_is_always_a_name_in_the_chain | the same chains | 21 s |

Planted bugs, in the real functions, one run each, reverted:

| Planted bug | Harness | Result |
|---|---|---|
| scopes_collide: shared marker ignored on the right | `scopes_collide_up_to_5_bytes` | caught, 41 s |
| overlaps: match by text prefix, not component | `overlaps_follows_components_two_levels_deep` | caught, 169 s |
| overlaps: one direction only | `overlaps_is_symmetric_and_reflexive_up_to_4_bytes` | caught, 321 s |
| overlaps: one direction only | `overlaps_follows_components_two_levels_deep` | caught, 148 s |
| clean_bytes: a `.` segment is kept | `clean_path_is_normal_up_to_4_bytes` | caught, 148 s |

Not finished. A harness for `clean_path` itself, the wrapper that sizes a buffer with `vec!` and converts with
`String::from_utf8_lossy`, asserting that it returns the bytes `clean_bytes` computes: at 2 bytes for the path and the cwd
it ran 19 minutes without a result, at 1 byte 11 minutes (stopped by me, not a timeout verdict). A growing `String` is
out of the solver's reach (section 52.4). Not kept. Honest notes: the four harnesses passed on their first run, so the
plants are what show they can fail; none reaches the 3 level or 5 byte-path structures the tickets worry about, and the
`clean_path` wrapper is still covered only by the differential unit test.

| check_chain: a chain of one name accepted | `a_chain_needs_two_or_more_valid_distinct_names_and_the_first_fault_is_named` | caught, 524 s |
| check_chain: names are not checked | `a_chain_needs_two_or_more_valid_distinct_names_and_the_first_fault_is_named` | caught, 150 s |
| check_chain: repeats allowed | `a_chain_needs_two_or_more_valid_distinct_names_and_the_first_fault_is_named` | caught, 216 s |
| check_chain: a repeat is looked for only against the previous name | `a_chain_needs_two_or_more_valid_distinct_names_and_the_first_fault_is_named` | caught, 634 s |
| min_in: returns the higher label | `min_is_the_lower_known_label_and_none_for_an_unknown_one` | caught, 38 s |
| default_cap_in: the top of the chain | `the_default_cap_is_always_a_name_in_the_chain` | caught, 25 s |

Honest notes. The three harnesses passed on their first run, so the plants are what show they can fail. The chain harness
is slow because it compares strings pairwise twice (the function and the oracle); the plant that looks for a repeat only
against the previous name took 634 s to be caught, close to the 900 s default limit of `proto/scripts/kani`.
`LabelChain::parse` is still uncovered.

### 52.8c Update, 2026-11-18: the recovery code framing (#656, Kani 0.68.0, same machine, one harness at a time)

`keys::restore` and `recovery_code` still have no harness: they filter a `String`, decode base32 into a `Vec` and hash,
and the solver cannot finish that. What changed is that the decision "these bytes are a key" was split out into
`frame_with` and `unframe_with` (byte slices, the check passed in) and proved at 32 byte keys with a stand-in check.
It says nothing about SHA-256 or the base32 text.

| Harness | Bound | Time |
|---|---|---|
| a_recovery_code_of_the_wrong_length_is_malformed | 0 to 36 bytes, any content, any length but 34 | 7 s |
| the_check_decides_between_a_key_and_a_mistyped_code | any 34 bytes; Ok exactly when the check matches, then the first 32 bytes | 8 s |
| a_framed_key_round_trips_and_a_changed_check_byte_is_caught | any 32 byte key, any one changed check byte | 11 s |

Planted bugs (each in the real function, harness run, reverted; CAUGHT is `VERIFICATION:- FAILED`): the run of
2026-11-18:

| Planted bug | Harness | Result |
|---|---|---|
| unframe_with: length check accepts a longer input (!= to <) | `a_recovery_code_of_the_wrong_length_is_malformed` | caught, 10 s |
| unframe_with: only the first check byte compared | `the_check_decides_between_a_key_and_a_mistyped_code` | caught, 10 s |
| unframe_with: only the first check byte compared | `a_framed_key_round_trips_and_a_changed_check_byte_is_caught` | caught, 9 s |
| unframe_with: a mismatch is accepted | `the_check_decides_between_a_key_and_a_mistyped_code` | caught, 8 s |
| unframe_with: a mismatch is reported as Malformed | `the_check_decides_between_a_key_and_a_mistyped_code` | caught, 10 s |
| frame_with: the check bytes are those of the key with its first byte zeroed | `a_framed_key_round_trips_and_a_changed_check_byte_is_caught` | caught, 11 s |
| unframe_with: the key is returned shifted by one byte | `the_check_decides_between_a_key_and_a_mistyped_code` | caught, 10 s |

Honest notes. The harnesses passed on their first run, so the plants are what show they can fail. The stand-in check is
not SHA-256: a framing bug that only shows with the real hash would not be seen, and the typo guarantee for a changed key
byte is the real hash's, outside this proof.
### 52.8a Update, 2026-11-18: valid_name, valid_kind, valid_scope over bytes and at every length (#657, #664; Kani 0.68.0)

`valid_name`, `valid_kind` and `valid_scope` now delegate to byte folds (`safe_name_bytes`, `scope_bytes_ok`), which the
solver finishes on arrays; the 4 character harnesses stay and still call the real functions.

| Harness | Bound | Time |
|---|---|---|
| name_and_kind_bytes_accept_exactly_the_safe_alphabet_up_to_12_bytes | any 12 bytes, any length 0 to 12; accepted exactly when non-empty and every byte in the set (the set written by range in the harness, a second way) | 41 s |
| scope_bytes_accept_exactly_the_scope_alphabet_up_to_12_bytes | same, the scope set; `*` alone refused | 9 s |
| the_length_limits_hold_at_every_length_up_to_90 | a run of `a`, every length 0 to 90, through `valid_name`, `valid_kind` and `valid_scope` (40, 40, 80) | 99 s |
| one_symbolic_byte_in_a_full_length_run_decides_the_result | 40 bytes (name) and 80 bytes (scope) of `a` with one symbolic byte at a symbolic position | 14 s |

Planted bugs, in the real functions, one run each, reverted:

| Planted bug | Harness | Result |
|---|---|---|
| safe_name_bytes: space allowed | `name_and_kind_bytes_accept_exactly_the_safe_alphabet_up_to_12_bytes` | caught, 7 s |
| safe_name_bytes: space allowed | `one_symbolic_byte_in_a_full_length_run_decides_the_result` | caught, 12 s |
| safe_name_bytes: non-ASCII bytes allowed | `name_and_kind_bytes_accept_exactly_the_safe_alphabet_up_to_12_bytes` | caught, 7 s |
| safe_name_bytes: limit 41 instead of 40 | `the_length_limits_hold_at_every_length_up_to_90` | caught, 107 s |
| scope_bytes_ok: limit 79 instead of 80 | `the_length_limits_hold_at_every_length_up_to_90` | caught, 99 s |
| scope_bytes_ok: limit 79 instead of 80 | `one_symbolic_byte_in_a_full_length_run_decides_the_result` | caught, 6 s |
| scope_bytes_ok: star added to the alphabet (and the explicit refusal kept) | `scope_bytes_accept_exactly_the_scope_alphabet_up_to_12_bytes` | caught, 9 s |

Honest notes. The four harnesses passed on their first run, so the plants are what show they can fail. Strings of 13 to 39
bytes that mix characters are still only covered by the 12 byte and the one-bad-byte harnesses together, which is not
the same as all mixed strings of that length. The valid_scope line that refuses `*` explicitly is redundant with the
alphabet (#664) and is kept with a comment; a plant that removes it still cannot be caught.

## 54. Agent-tool subagents against launched sessions, and what the Agent tool gives a coordinator (desk research, 2026-11-21, tickets #581, #582, #583)

Question: for a team of coding agents, how do Agent-tool subagents differ from separate `claude -p` sessions started by `proto/scripts/team`, and what does the Agent tool offer a coordinator that klatch should or should not duplicate?

Method and limit: this section is read from sections 1 to 9 (documentation, marked DOCUMENTED there), section 50 (a live run of launched sessions) and the dogfooding tickets named below. It adds NO new live run. The questions of #581 that need one (what a subagent inherits, the exact SubagentStart input, whether a hook can inject context at start) stay open, see 54.4.

### 54.1 Subagents against launched sessions (answers the comparison asked in #582)

| Axis | Agent-tool subagent | Launched session (`proto/scripts/team`, section 50) |
|---|---|---|
| Identity | shares the parent's session id; under `--discover` it resolved to the parent's member until #503, which now refuses it when it is its own `claude` process and demands `KLATCH_NAME`; an in-process subagent is still not detected (#1152) | its own member through a worker credential (`KLATCH_IDENTITY_FILE`), verified (section 50.8) |
| Hook delivery | no SessionStart text (#491); hooks fired by its tool calls carry the parent's session id and an agent id and are silent (#493, #505) | every hook runs with the worker's own settings; mail arrives at tool boundaries (50.3) |
| Waiting for mail | none of its own | `klatch wait` inside the turn; a finished turn cannot be woken (#591), so `team resume` relaunches on unread mail (#1011) |
| Cost control | the parent's budget, no per-agent cap known | `--max-budget-usd` per worker, `TEAM_MAX_TOTAL_USD` team cap, wall clock watchdog (#593) |
| Isolation | optional `isolation: worktree` (section 6), restricted shell seen in dogfooding | own worktree and own settings by construction |
| Result to the coordinator | arrives only when it finishes (#583) | a room post and a log file; the board and claims show live state |
| Cost per session | not measured | about $0.003 and 9 s for a trivial Haiku task (50.3) |

### 54.2 What the Agent tool gives a coordinator (answers #583)

DOCUMENTED (sections 1, 2, 6, 8): `SendMessage` resumes a finished agent with its transcript and takes `notify_when_idle`; `ListAgents` shows subagents, teammates and sessions with a busy, idle or shell status; agent teams add a shared task list, teammate messaging, idle hooks and a lead; running agents hold a worktree lock. OBSERVED while coordinating about 20 subagents (the ticket): results only at finish, no live view of what an agent is doing except its room posts, repeated shell restrictions, two agents on one design item number, worktrees vanishing mid-task. Not found in the notes: a cancel or budget control per subagent.

### 54.3 Which of those klatch already provides, and which was klatch's to prevent

| Need | In klatch today |
|---|---|
| Live status | status words in the room (`working:`, `parked:`, `finished:`) and the board |
| Work ownership | claims with leases, file and ticket scopes (section 51); append-only sharing (#533); renewal on hooks |
| Work requests | asks |
| Delivery proof | receipts (#546) |
| Duplicate design item numbers | design fragments named by ticket (#502, #807) |
| Hook echo and empty lobby surprises | #493, #500, fixed |
| Router and facilitator roles | #189, #190 |
| Launch, budget, relaunch | `proto/scripts/team` (#593, #1011) |

Ranked remaining features, each already ticketed or noted: (1) a signal that tells an in-process subagent from its parent (#1152, blocked on 54.4); (2) one claim deadlock refusal with no cycle (#543, open); (3) a coordinator that assigns tickets from a backlog: not built, and the human has so far played it; no ticket is filed here because #189 and #190 closed without deciding it and the decision belongs to the human.

### 54.4 Not done, needs a real run

A live run on the installed Claude Code, throwaway settings, isolated KLATCH_HOME, recording: the standard input of SubagentStart and SubagentStop hooks, whether `agent_id` or any agent variable reaches a command run by an in-process subagent (this decides #1152), what a subagent inherits (settings, hooks, environment, permission mode, MCP), and subagent lifetime against the parent turn. A subagent run needs a model session, which the launched builders do not start.

## 55. Subagents and hooks, measured in a live headless run (2026-11-21, installed Claude Code, ticket #581)
Results of a live run for #581 and #1152: headless `claude -p --model haiku --setting-sources local`, command hooks for SubagentStart, SubagentStop, PreToolUse, PostToolUse, Stop, UserPromptSubmit and SessionStart, three sessions, about 0.009 USD in all, no klatch in the run. This is the measured part of the subagent questions; it is cited as "the research note section 55" until assembled (it extends the subagent discussion that section 54 of the plan refers to).

### 55.1 SubagentStart and SubagentStop hook inputs

- SubagentStart: `agent_id`, `agent_type`, `cwd`, `hook_event_name`, `prompt_id`, `session_id`, `transcript_path`. No `permission_mode`.
- SubagentStop: `agent_id`, `agent_type`, `agent_transcript_path` (basename `agent-<agent_id>.jsonl`), `transcript_path` (the parent's), `session_id` (the parent's), `permission_mode`, `last_assistant_message`, `stop_hook_active`, `background_tasks`, `session_crons`, `effort`, `cwd`, `prompt_id`, `hook_event_name`.
- A subagent's own tool calls: PreToolUse and PostToolUse carry `agent_id` and `agent_type` besides the usual fields; `session_id` is the parent's. The parent's own tool calls, including its Agent call, have neither key.
- Order seen: PreToolUse(Agent), SubagentStart, the subagent's PreToolUse and PostToolUse for Bash, SubagentStop, PostToolUse(Agent), the parent's Bash, Stop.

### 55.2 What a subagent inherits

- Hooks: yes. The session's hooks fire for the subagent's tool calls (that is how `agent_id` reaches them); SubagentStart and SubagentStop fire from the parent's settings. No SessionStart or UserPromptSubmit fires for the subagent (each fired once, for the parent).
- Permission mode: yes. The subagent's hook inputs showed the parent's `acceptEdits`, and the Bash allow rule applied to the subagent as well.
- Environment: identical to the parent's. The variable names match, `CLAUDE_CODE_SESSION_ID` is the parent's, no agent id variable exists (`CLAUDE_CODE_AGENT_ID` is unset), and the process chain is the same `claude` process with no process of its own for the subagent. So a command a subagent runs cannot learn that it is a subagent from its environment or its parents; only a hook can (see the design item 284).
- Working directory: the same project directory.

### 55.3 SubagentStart additionalContext reaches the subagent

A SubagentStart hook that printed `{"hookSpecificOutput":{"hookEventName":"SubagentStart","additionalContext":"..."}}` reached the subagent: it replied with a marker word it could only have learned from the hook. A per-subagent convention text or identity line can therefore be injected at start, and the hook knows `agent_id` and `agent_type` to build it.

### 55.4 Foreground versus background lifetime

- Foreground (the default): the subagent starts and stops inside the parent's turn; SubagentStop comes before the parent's Agent PostToolUse and Stop.
- Background (`run_in_background` true): the parent's Stop fired before SubagentStop (15:49:01 and 15:49:02 in the run). The subagent outlives the parent's turn, and a completion notification then starts a second parent turn (a second UserPromptSubmit and Stop). In `-p` mode the process stayed alive until that second turn finished.

### 55.5 Not run

- Isolation modes (worktree and remote subagents).
- Agent teams.
- MCP server inheritance by a subagent.

## 56. Turso Database as a Store backend: spike findings (2026-11-21, ticket #620)
Question: could a Turso Database (the Rust rewrite of SQLite, MIT, beta) sit behind the `Store` trait, pass the conformance suite, survive 16 processes and encrypt at rest with a key from klatch's key custody? This was a spike to learn, not to ship: the code lives in `experiments/turso-spike/`, a standalone crate (its own `[workspace]`, its own `Cargo.lock`, path dependency on `klatch`). It is not a member of `proto/`, CI does not build it, and the main `Cargo.lock`, `deny.toml`, binaries and release artifacts are unchanged. The measurements are one run on one machine (macOS, Apple silicon, release build) with small N; read them as orders of magnitude.

### 56.1 Can it be built here

Yes. `turso` 0.9.0-pre.6 (MIT, pre-release, pinned with `=`) fetches from crates.io and builds with stable Rust; the whole spike with `klatch` builds in about three minutes in debug and the release binary is 32 MB (it links both SQLCipher and Turso). There is no encryption feature flag to enable at build time: encryption is a runtime option, `Builder::experimental_encryption(true)` plus `with_encryption(EncryptionOpts { cipher, hexkey })`. Ciphers in 0.9.0-pre.6: aes128gcm, aes256gcm, aegis128l, aegis128x2, aegis128x4, aegis256. The spike uses aegis256 with the 32 bytes of a klatch `Key` hex encoded, so key custody needs no change to feed it.

Dependencies: `turso` adds about 300 crates to the tree (klatch alone is about 70), including `mimalloc` (a C allocator), `chrono` and `turso_sync_engine`/`hyper`/`tokio` machinery that the `sync` feature gates but the lockfile still resolves. Licences are all permissive (MIT, Apache-2.0, BSD, Zlib, Unicode-3.0, ISC and dual licences that include one of those; `cfg_block` has no licence field). `cargo deny` with `proto/deny.toml` on the spike: licenses ok, bans ok, sources ok, advisories FAILED: `chrono` 0.4.45 is flagged unmaintained (RUSTSEC-2026-0543, "all versions are affected"), pulled in by `turso_core`. The main workspace sets `all-features = true` and denies, so adding the dependency there would fail its `cargo deny` job today. That is the reason the spike is standalone.

### 56.2 Conformance

The suite is `proto/klatch/src/store/conformance/` (108 behaviours today, not the 90 the ticket counted), wired for SQLite in `sqlite/tests/conformance.rs`. The spike includes both files from the klatch source by `#[path]` and runs them unchanged against `TursoStore` (a file in a temp directory, encrypted with aegis256), so the suite cannot drift from the copy. `TursoStore` ports the required methods and these areas: messages (post, read, look, peek, wait and wake positions, hook read, tail, head, advance cursor), members (list, rooms of, mute), plain claims (conflict, expiry, owner reclaim, scopes), policy, audit, refusals and credentials. It keeps klatch's SQLite schema for the tables it uses. It does not port sealed labels, claim waits and deadlock notes, claim changes, anchors, asks, reviews, sessions, member notes, parking, pruning or room state.

Result: 65 of 108 pass, 43 fail.

- Pass: messages 32 of 33, claims 11 of 23, members 5 of 15, asks 7 of 16, policy 6 of 6, credentials 3 of 3, anchors 1 of 4.
- Fail (43): every failure is a method the spike did not port, so the trait default answered "nothing" or "this store does not keep that" (claims 12, members 10, asks 9, reviews 7, anchors 3, pruning 1, plus the one test that checks a store declares every feature). No failure was traced to something Turso cannot do. The SQL the ported areas use (upsert with `ON CONFLICT`, `BEGIN IMMEDIATE`, scalar `MAX`/`MIN`, `COALESCE`, `substr`, `AUTOINCREMENT`, unique indexes, BLOB columns, correlated subquery) all ran. Porting the rest is mechanical work on the same SQL; it was left out because the spike had answered the questions that matter.
- Not tried, so unknown: `PRAGMA user_version` based migrations and `sqlite_master` introspection (the SQLite store's migration path), and the backup and archive paths (DESIGN item 30, `VACUUM INTO`, `ATTACH`), which in Turso sit behind `experimental_attach` and `experimental_vacuum`.

### 56.3 Encryption at rest (the point of the spike)

`turso-spike inspect` writes a known marker and looks at the files.

- Turso with aegis256: the file starts with the bytes `Turso` and a version, not `SQLite format 3`; the marker appears in neither the database file nor its WAL; a wrong key fails at open with `Decryption failed for page=1`; opening with no key fails with `file is not a database`.
- Turso without a key: the file is an ordinary SQLite file (`SQLite format 3`) and the marker is plainly visible in the WAL. Encryption is opt-in per file.
- Interchange: klatch's SQLCipher store refuses the Turso file (encrypted or plain) and Turso refuses the SQLCipher file (plain open: `file is not a database`; with a key: `invalid page size in database header`). Neither engine can read the other's encrypted files, as the research expected.
- The header is not hidden: an observer can tell a file is a Turso database. SQLCipher hides the header too. Whether that matters under klatch's zero trust maxim is a human call.
- Turso's encryption is behind `experimental_encryption`, and Turso describes it as experimental.

### 56.4 Several processes on one file

`turso-spike race` starts 16 processes that each open the same file and post to one room, then a fresh open checks that every post is stored with dense `seq` and unique ids.

- Default Turso open: only the first process can open the file; the other 15 fail at open with `Locking error: Failed locking file ... File is locked by another process`. That breaks every klatch use that has a CLI process, a hook process and a server touching one store.
- With `experimental_multiprocess_wal(true)`: 16 processes by 50 posts and, three times, 16 by 300 posts all succeeded (4800 written, 4800 stored, dense seq, unique ids, no failed post and no busy error). This is a small run, not a proof; the flag name says experimental and the ticket's source said several processes on one file can break. A real verdict needs the bench's 16-process race at its full size, crash injection (kill -9 mid transaction) and a soak, none of which were done.
- SQLCipher store in the same race: 4800 of 4800, 1.14 s; Turso with multiprocess WAL: 1.5 s (three runs, 1.49 to 1.57 s).

### 56.5 Speed

`turso-spike bench <dir> 300`, microseconds per call, file backed, release build, one process, so single-run noise is large:

| backend | post | read (post then read) | wait poll (nothing new) | tail 20 |
|---|---|---|---|---|
| SQLCipher (klatch store) | 161 | 273 | 57 | 283 |
| Turso, aegis256 | 162 | 249 | 120 | 32 |
| Turso, plain | 201 | 253 | 132 | 29 |
| Turso, aegis256, multiprocess WAL | 171 | 278 | 129 | 31 |

Posts and reads are on par; a wait poll costs about twice as much (the spike's `touch` is a write on every poll; the SQLite store skips it when already seen that second), and a plain read of 20 rows is faster on Turso. After 600 messages the Turso files held about 2.8 MB (database plus WAL) against 316 KB for SQLCipher, probably because the WAL is checkpointed less eagerly (not investigated). A debug build of the suite is slow (about 12 ms a post), so do not judge speed from `cargo test`.

### 56.6 Migration and archive

Migration from an SQLCipher store through the Store trait works for messages: 500 messages read with `tail` from the SQLCipher store and written with `post_as` to the Turso store in 0.09 s, with `seq`, sender and body identical. Message ids are not kept, because `post_as` mints a fresh id; the trait has no way to import a message with its id, so a migration or an archive restore through the trait changes ids (ticket #1217). The archive and restore items (DESIGN item 30) and the rekey and seal paths are SQLite specific (files, `VACUUM INTO`) and would need a Turso equivalent; they were not attempted.

### 56.7 Could Rust verifiers cover more

Kani and Shuttle model klatch's own Rust, which is the same on either backend. A Turso backend would remove the C layers (SQLite, SQLCipher, vendored OpenSSL) from klatch's own build, but it adds a large Rust dependency that no verifier here covers (about 300 crates), so the verified surface would not grow. Nothing in the spike changes the existing proofs.

### 56.8 Recommendation

Stay on SQLite, SQLCipher and OpenSSL, as the human decided on 2026-11-15, and keep #620 as the revisit marker. Reasons, in order of weight: (1) multi-process use needs an experimental flag and was only lightly exercised; (2) encryption is experimental, hides no header and makes files that neither engine reads; (3) the dependency set adds an unmaintained crate (`chrono`) that fails `cargo deny` as configured and about 230 more crates than today; (4) no data on crash safety. In favour: the trait seam held (a second engine passed 65 behaviours with 744 lines of backend and no change to klatch), speed is comparable, and encryption with klatch's own key works. Revisit when Turso's encryption leaves experimental, multi-process WAL is documented as supported, and `chrono` is gone from `turso_core`, or at the pre-1.0 review. To resume: port the unported areas in `experiments/turso-spike/src/turso_store.rs` and rerun `cargo test` there (the suite will say what is left), then `turso-spike race` at full size with `kill -9` injection.

### 56.9 How to rerun

From `experiments/turso-spike/` (set `CARGO_TARGET_DIR` outside the repository; delete it afterwards): `cargo test` runs the conformance suite on `TursoStore` (`TURSO_SPIKE_PLAIN=1` skips encryption, `TURSO_SPIKE_DIR` picks the directory); `cargo run --release -- inspect|bench|race|migrate <dir> ...` runs the measurements (`cargo run --release -- help` lists them).

## 57. ACP specification check for the librarian agent (2026-11-20, ticket #869)
The Agent Client Protocol pages at agentclientprotocol.com were read on 2026-11-20 and compared with `klatch acp --stdio`. The per-item comparison is in the design item 188.

### 57.1 What the spec fixes and what it leaves open
- Fixed: integer `protocolVersion` (1), `agentCapabilities` booleans default to unsupported, a `sessionId` from `session/new`, `agent_message_chunk` updates, the `stopReason` list, `session/cancel` as a notification answered by a `cancelled` stop reason on the prompt, one message per line on stdio with nothing else on stdout.
- Open: what `authenticate` answers when no method is advertised; whether a refused query should be `refusal` or text with `end_turn`; whether a missing or unknown `sessionId` is invalid params or resource not found (-32002).
- Optional: `session/load`, slash commands, plans, modes.

### 57.2 Limits of this check
The check is of published text, read through a page summarizer, not of the JSON schema file itself and not of any editor. No claim is made that Zed or JetBrains accept the agent.
