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
- **NOT OBSERVED.** A multi-step exchange in the room of two: alpha's ask already said accents should be transliterated, so the helper had nothing to ask back. The scenario did not force a question (#210 stays open). Two helpers racing within one poll; a third helper; whether agents would use any of this unprompted (#209).

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
- **NOT OBSERVED.** More than one helper, declines, the dual role (#246), a session that ignores the convention, longer lobbies.

## 32. A deadlock trap: opposite-order claims (2026-11-13)

Run fourteen (#246). Two real agents, one shared directory with `X.py` and `Y.py`, convention taught only by the SessionStart hook, task text only saying to claim a file before editing it. Alpha must edit X then Y and keep its claim on X until Y is done; beta must edit Y then X and keep its claim on Y until X is done: hold-and-wait in opposite order.

- **OBSERVED.** The deadlock formed for real: each claimed its first file and edited it, then each got a claim conflict on the other's file (the conflict message names the holder and the seconds left).
- **OBSERVED.** The agents resolved it by themselves in the lobby in about a minute (alpha 69 s, 18 turns, $0.237; beta 51 s, 15 turns, $0.202). Both posted the deadlock; the proposals crossed ("our messages crossed"); they agreed that the one who had already edited its first file goes first; the other reverted its edit and released; the first finished both and released; the second then did its own. No human involved, no claim lapse needed.
- **OBSERVED.** The final files say `from alpha` in both: the second agent's later edits replaced the first agent's. Nothing in klatch (or in my task text, which required both changes "to land together") made the first agent's result survive; the claims prevented concurrent edits, not a sequence of overwrites.
- **OBSERVED.** Crossed proposals: each offered to yield to the other at the same moment. A deterministic tie-break was not available to them and cost a message round.
- **NOT OBSERVED.** Three or more members in a cycle; a cycle through asks (A waits on B's answer, B waits on A's) rather than through claims; a member that is busy for minutes and cannot answer; agents that do not negotiate (a crashed holder, a claim that outlives its owner by the 600 s TTL).

### 32.1 The same trap with deadlock detection (run fifteen)

Same scenario, binary with detection (DESIGN section 25.12) and the convention line about conflicts.

| | Turns | Cost | Time | Lobby messages about the deadlock |
|---|---|---|---|---|
| Run fourteen (no detection) | 33 (18 and 15) | $0.44 | 69 s and 51 s | 6, with crossed offers to yield |
| Run fifteen (detection) | 25 (18 and 7) | $0.34 | 108 s and 91 s | 0 |

- **OBSERVED.** The refusal that closed the cycle carried the deadlock text, and that agent (alpha) did what it said: restored `X.py` to `initial`, released, and waited; the other agent (beta) finished both files and released. No negotiation was needed and nothing was posted about the deadlock.
- **OBSERVED.** Fewer turns and less cost, but longer wall time (beta held its first claim and waited about a minute on purpose, because its task said the two changes must land together; it did not follow the convention line about releasing before waiting). One run each: the turn and cost drop could include variance; the absence of negotiation messages is a direct effect.
- **OBSERVED, still true.** Both files end up `from alpha`: the second agent's edits replaced the first's. Detection resolves the wait, not the semantic conflict.
- **NOT OBSERVED.** Three or more members; a cycle through asks; a crashed holder.

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

## 38. Peer identity from a gateway-owned socket (2026-11-13, macOS 14, experiment 14.3, ticket #29)

`klatch-harness/tests/peer_identity.rs` shows it works. A gateway that owns a Unix socket can ask the kernel for the pid of the process at the other end of an accepted connection (`getsockopt` with `LOCAL_PEERPID` on macOS, `SO_PEERCRED` on Linux; the Linux branch is written but was not run here). That pid is the connecting process, here the script a session's shell started, not the session itself; the existing ancestry walk (`find_session`, with the same liveness checks) then maps it to the session's registry record, giving the session id, name and directory. A process with no session above it maps to nothing and stays unattributed.

What this gives and does not give: the identity is as strong as the process tree and the registry, which are same-user evidence (a process can lie about its ancestry only by being the user, who can do anything anyway). It is not proof against another process of the same user that arranges to descend from a session. It needs a gateway that owns a socket, which klatch does not have yet (the CLI opens the store directly); it is the identity source for that daemon, when it exists (epic #152), where the socket's file mode also admits only its owner.

