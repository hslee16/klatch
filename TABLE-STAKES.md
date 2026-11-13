# Table stakes for agent rooms

Date: 2026-11-12. Derived from `SURVEY.md`. Read that file first for what each system is, what was
verified and what was not.

"Table stakes" here means: **features a newcomer must ship to be taken seriously.** The list is derived
from evidence, not from taste. It is organised by what the evidence says, in four groups, and two of them
are called table stakes for different reasons:

- **Market table stakes** (section 3): what most existing tools already ship. Omit these and users compare
  you unfavourably on day one.
- **Table stakes by consequence** (section 6): what almost nobody ships, but whose absence has a
  *documented* cost (an incident, a published demonstration, a practitioner failure). A room meant to be
  trusted across repositories cannot omit these even though the market does.
- **Emerging expectations** (section 4): shipped by the best two to five tools; plan to have them at v1.
- **Differentiators** (section 5): shipped by at most two; this is where a new room can be better, and
  note that group messaging itself is one of them.

Section 7 lists anti-features to avoid, section 8 scores the current design, and section 9 gives the
minimum set for a first release.

## 1. Method and its limits

**Reference set.** Twelve systems that exist to let coding-agent sessions coordinate, as covered in
`SURVEY.md`:

| Code | System | Code | System |
|---|---|---|---|
| NS | Claude Code cross-session messaging | CP | claude-peers-mcp |
| AT | Claude Code agent teams | AQ | agent-message-queue (AMQ) |
| AM | mcp_agent_mail (Rust rewrite) | PR | claude-presence |
| CO | Cotal | MS | Multisession |
| AL | agent-loom | CR | claude-relay |
| AB | agent-bus | MC | mcp-coordinator |

Large orchestrators (Gas Town, ruflo), spawn-and-supervise managers (Claude Squad, agent-deck and
the others in `SURVEY.md` section 3.9), protocols and infrastructure are **not** in the set, because
they are not rooms for independently started sessions.

**Scoring.** `Y` the feature is shown in the README or docs; `P` partial or conditional; `N` shown to
be absent; `?` not shown (**counted as "not shown", not as absent**). The evidence is README level and
mostly one reader's reading (see the verification tags in `SURVEY.md`). Nothing was run.

**Tier thresholds.** Table stakes: `Y` in at least 6 of 12. Emerging: `Y` in 3 to 5. Differentiator:
`Y` in 0 to 2.

**Limits, stated plainly.**

- The sample is twelve, drawn from open-source GitHub projects and Anthropic's own features. A
  different sample (closed products, enterprise platforms) would move the counts.
- Several of the twelve are one person's weekend project (stars 0 to 12). They bias the sample
  toward what is easy to build, which is why "history for latecomers" and "claims" look rarer than
  they might be in mature products.
- `?` cells hide real features. Treat low counts as "rare in what is documented", not "does not
  exist".
- The matrix says what exists, not what is good. Cotal's channels and agent-loom's receipts are
  better designed than their `Y` suggests relative to a README-only `Y`.

## 2. The matrix

| Feature | NS | AT | AM | CO | AL | AB | CP | AQ | PR | MS | CR | MC | Y | P |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Named member identity | Y | Y | Y | Y | Y | Y | Y | Y | Y | P | Y | Y | 11 | 1 |
| Directed (one-to-one) messages | Y | Y | Y | Y | Y | Y | Y | Y | P | Y | Y | N | 10 | 1 |
| Local-first, no cloud account | Y | Y | Y | P | Y | Y | Y | Y | Y | Y | P | Y | 10 | 2 |
| Works for independently started sessions in different repos | Y | N | Y | P | Y | Y | Y | P | N | Y | Y | N | 7 | 2 |
| Uses only documented Claude Code features | Y | Y | Y | P | P | Y | N | P | Y | Y | P | P | 6 | 5 |
| Human-readable record or UI | P | P | Y | Y | ? | Y | N | Y | ? | Y | ? | ? | 5 | 2 |
| Presence or roster | Y | Y | P | Y | ? | ? | Y | N | Y | N | ? | P | 5 | 2 |
| Honest trust-model statement in the docs | Y | ? | Y | P | ? | ? | ? | Y | ? | ? | ? | Y | 4 | 1 |
| Persistent history a latecomer can read | N | P | Y | Y | P | Y | P | P | N | P | P | ? | 3 | 6 |
| Delivery receipts or status | P | ? | Y | ? | Y | ? | N | Y | N | P | ? | ? | 3 | 2 |
| Group message (broadcast or channel) | N | P | P | Y | ? | ? | N | N | Y | N | ? | N | 2 | 2 |
| Advisory claims or leases on paths or resources | N | P | Y | N | Y | ? | N | N | P | N | N | P | 2 | 3 |
| Expiry (TTL) on claims | n/a | ? | Y | n/a | ? | n/a | n/a | n/a | ? | n/a | n/a | ? | 1 | 0 |
| Mute, attention modes or mentions | N | ? | ? | Y | ? | ? | N | ? | ? | ? | ? | ? | 1 | 0 |
| Loop or burst throttling | Y | ? | ? | ? | ? | ? | ? | ? | ? | ? | ? | ? | 1 | 0 |
| Sender identity that is verified, not declared | P | P | N | P | P | N | N | N | N | N | P | P | 0 | 6 |
| Sensitivity labels, clearance or a scrubber | N | N | N | N | N | N | N | N | N | N | N | N | 0 | 0 |

Notes on cells that carry judgement:

- **NS verified sender = P.** The socket is restricted to the operating-system user, and an own-child
  message is verified by process evidence, but the sender's permission mode is self-declared
  (`SURVEY.md` section 7.3).
- **AL verified sender = P.** It stamps an identity only when it can prove process ancestry and
  labels everything else `[label; not a reply address]`. Better than the others; still conditional.
- **CO and group messaging, Y.** Named channels with multicast, plus unicast and anycast.
- **AM group messaging, P.** Project-level broadcast exists; a later joiner does not see older
  broadcasts; there are no named rooms.
- **CP local-first = Y.** The core is a localhost broker; an optional feature sends context to a
  third-party API when a key is set, which belongs in the anti-features list (section 7).
- **Labels: no system in the set has them.** A large orchestrator (ruflo) *claims* a PII pipeline
  and trust tiers in its README (unaudited, aimed at agents on different hosts); it is outside the
  reference set and does not change the count.

## 3. Market table stakes (Y in at least 6 of 12)

Each item lists what "good enough" looks like. These are the minimum a newcomer must match.

### TS1. A stable, named identity for each member (11 of 12)

Every system gives an agent a name. **Good enough:** a name that survives a restart, shown on every
message, unique within a room, and distinct from a display label. **Pitfall seen:** in Claude Code the
session name is a label, not a credential, and a name changes when a session is restarted
(`CASE-STUDY.md`). **Design hook:** member id independent of name (R16).

### TS2. Directed messages (10 of 12)

One-to-one delivery to a named member. **Good enough:** reaches a busy or idle session without the
sender knowing which; a clear refusal when the target is gone. **Design hook:** `to` field, plus
delivery through hook injection and the nudge path.

### TS3. Local-first operation with no account and no cloud (10 of 12)

Everything runs on the user's machine and works offline. **Good enough:** no sign-up, no data leaves
the machine by default, no telemetry on by default, installation that does not require piping a remote
script to a shell without verification. **Pitfalls seen:** default-on telemetry (Cotal's CLI), an
optional call to a third-party model with the user's context (claude-peers), a remote installer script
(several). **Design hook:** a design principle; add it to `DESIGN.md` goals as an explicit non-feature
list.

### TS4. Works with independently started sessions in different repositories (7 of 12)

Not only with sessions the tool launches itself. **Good enough:** a session started in any terminal
can join by an explicit command or by installing hooks, without relaunching. **Pitfall seen:** several
tools require a development flag or a launcher. Cotal's local mode says to launch through its own
`spawn`. **Design hook:** join through the CLI plus hooks, no relaunch (`DESIGN.md` 8.6, 8.8).

### TS5. Built from documented Claude Code features (6 of 12)

Hooks, slash commands, MCP, Bash, cross-session messaging. **Good enough:** nothing that depends on a
research-preview flag for its core function, and a self-test that reports loudly when a Claude Code
release changes the behaviour it depends on. **Pitfall seen:** projects that depend on channels report
that push does not load on a stock install. **Design hook:** R18, R19 and the canary in section 12 of
`DESIGN.md`.

## 4. Emerging expectations (Y in 3 to 5)

The best two or three systems ship these; the rest will probably follow. Plan to have them at v1.

| Item | Count | What the best implementation does | Who |
|---|---|---|---|
| Human-readable record or UI | 5 | A Markdown or plain-text transcript regenerated from the log; a console | AM (Git and web UI), CO (console), AQ (plain files), MS (Markdown), AB (live view) |
| Presence or roster | 5 | Who is here and what state they are in, with a heartbeat that says "alive", not "making progress" | CO, PR, NS, AT, CP |
| Honest trust-model statement | 4 | A security file that says plainly it is **not an isolation boundary** between untrusted agents | AQ, AM, MC, NS |
| History for a latecomer | 3 (plus 6 partial) | A latecomer can read prior messages or a pinned summary | AM, CO, AB |
| Delivery receipts | 3 | Distinguishes queued, delivered, drained and read. "A handover closes on a receipt" | AQ, AL, AM |

**Gap in the current design:** `DESIGN.md` has no first-class **receipts**. Add a receipt model (see
section 8).

## 5. Differentiators (Y in 0 to 2): rare in what exists

These are where a new room can be better than what exists. Several are exactly the reason to build.

| Item | Count | Why it is rare | Why it matters |
|---|---|---|---|
| Group messaging with named rooms | 2 | Point-to-point is easier to build and to secure | A room is the whole point of this project; but note it is a differentiator, not table stakes |
| Claims or leases with owner, reason **and expiry** | 2 (AM, AL) | Needs a store and a policy; agents forget to release | The top collision cause in practitioner accounts (`SURVEY.md` section 8) |
| Mute, attention modes, mentions | 1 (CO) | Needs delivery filtering | Context cost scales with every agent that talks |
| Verified sender identity | 0 (6 partial) | Claude Code gives no stable session credential; the sender's mode is self-declared | Impersonation and the "trusted teammate" framing (`claudemessaging` F-1 and F-3) |
| Sensitivity labels, clearance and a scrubber | 0 | Needs a policy model and a human decision point | The documented exfiltration path (`SURVEY.md` section 7.1 and 7.2) |
| Loop and burst throttling | 1 (NS) | Receiver-side detail | Message loops are a reported failure; the native feature already throttles |

A caution that applies to this whole table: a differentiator is only valuable if it is also
*usable*. Cotal's mute mode and agent-loom's receipts are good ideas that cost little to adopt;
labels and clearance are the one that costs real design effort.

## 6. Table stakes by consequence (rare in the market, costly to omit)

The market does not ship these, but a room that crosses a public and a private repository **cannot
omit them**. Each has a documented source for why.

| # | Requirement | Evidence that omission is costly |
|---|---|---|
| C1 | **Claims expire.** A lease with no expiry is a lock, and agents forget to release | Cursor: agents "would hold locks for too long, or forget to release them entirely" (V-PAGE) |
| C2 | **Claims are advisory, with a commit-time or hook-time check that makes them bite** | `mcp_agent_mail` pre-commit guard; a practitioner argues for hard enforcement through a `PreToolUse` hook (REPORT) |
| C3 | **Peer text is delivered as fenced data with provenance, never as authority** | CodeIntegrity: "The channel carries a request without preserving the original request's provenance or authorization context" (V-PAGE) |
| C4 | **A content boundary, not only a topology boundary** (labels and clearance) | Issue #92060: cross-folder content moves through `SendMessage` with no prompt (V-PAGE); the contact gate in `mcp_agent_mail` controls who may message whom, not what they may say |
| C5 | **The human can read everything and stop everything** | METR: about 1,200 agents used an unsanctioned message board; 700 took part in an attack (V-PAGE page, summary only) |
| C6 | **No member can add itself to a room, raise its own clearance or open a new channel** | Same incident; and the sender's declared mode controls the hold gate (`claudemessaging` F-3) |
| C7 | **A receiver-side budget:** size cap, burst limit, repeat suppression, bounded queue | The native feature already does this; a room must at least match it or it becomes the weaker path |
| C8 | **An honest statement of what the system does not protect against** | AQ's SECURITY.md and `mcp_agent_mail`'s authentication notes; the field expects it |
| C9 | **Audit of refusals as well as acceptances** | The CSA and OWASP notes ask for an attributable audit trail (REPORT, SNIPPET) |
| C10 | **A clear license** | claude-relay has none and cannot be adopted; `mcp_agent_mail` is "MIT plus a rider" and the API reports NOASSERTION |

## 7. Anti-features: things that exist in the field and must not be copied

Each is a documented behaviour of a surveyed tool.

1. **Typing into a terminal to wake an agent** without a guard against open dialogs (AMQ's default
   doorbell can answer a permission dialog; its optional dialog hook exists for that reason).
2. **A documented launch that skips permission prompts** (`--dangerously-skip-permissions` in
   claude-peers' quick start; auto-accept modes in orchestrators).
3. **A development flag as the normal way to run** (`--dangerously-load-development-channels`).
4. **An unauthenticated localhost broker** that anything on the machine can post to.
5. **Telemetry on by default.**
6. **Sending the user's working context to a third-party model** for a convenience feature, even
   optionally, without a clear off switch and a statement of what is sent (claude-peers' summaries).
7. **A relay that frames an unattributed message as a "trusted teammate"** (`claudemessaging` F-1).
8. **Trusting a self-declared permission mode** to decide what is held (F-3).
9. **Claims that block forever.**
10. **A tool that can purge another member's history** without a human step (claude-relay's admin
    tools).
11. **Installing by piping a remote script to a shell** with no verification step.

## 8. How the current design scores against this list

Mapping to `DESIGN.md` as of this survey. "Gap" means the design does not yet say it.

| Item | In the design? | Where | Action |
|---|---|---|---|
| TS1 identity | Yes | 8.4, R5, R16 | none |
| TS2 directed messages | Yes | 8.2 `to` | none |
| TS3 local-first, no telemetry | Implicit | goals | **Gap:** state it as a non-feature list |
| TS4 independent sessions, no relaunch | Yes | 8.6, 8.8 | none |
| TS5 documented features only | Yes | R18, R19, section 12 canary | none |
| Human-readable record | Yes | 8.1, R10 | none |
| Presence | Yes, thin | 8.7 `room who`, heartbeat in M2 | clarify "alive, not progressing" semantics |
| Honest trust statement | Partly | section 5 | **Gap:** a standalone `SECURITY.md` of limits |
| History for latecomer | Yes | R4, 8.6 | none |
| Delivery receipts | **No** | | **Gap:** add receipt states (queued, delivered, shown to the model, acknowledged) |
| Group rooms | Yes | whole design | differentiator, keep |
| Claims with owner, reason, expiry | Yes | 8.7, R1, R2 | none; add the commit-time check as an option |
| Mute, mentions | Yes | R14, 8.5 | none |
| Verified sender | Yes | 8.4, R5 | none; copy agent-loom's "unattributed" tag for anything unproven |
| Labels, clearance, scrubber | Yes | 10, R7 to R9 | the main differentiator; most effort |
| C1 to C3, C5, C6, C9 | Yes | 8.7, 10, R6, R11 | none |
| C4 content boundary | Yes | 10 | none |
| C7 receiver budget | Partly | 8.5 caps | **Gap:** add burst limit, repeat suppression and a bounded queue explicitly, matching the native feature's numbers |
| C8 limits statement | Partly | section 5 | same as above |
| C10 license | **No** | | **Gap:** choose a license before any code |
| Anti-features 1 to 11 | Mostly avoided by construction | 9, 8.8 | add the list as a checked non-goal |

## 9. The minimum set for a first release that will not be dismissed

If the first release ships only these, it is credible against what exists:

1. Named, stable members (TS1) and directed messages (TS2).
2. Rooms: group messages with a pinned status and history for a latecomer.
3. Local-first with no telemetry and a verified installer (TS3).
4. Join by CLI and hooks, no relaunch (TS4), using documented features and a self-test (TS5).
5. A plain-text transcript the human can read, a pause switch and a close command.
6. Claims with owner, reason and **expiry**, advisory, plus a way to check before a wide command.
7. Fenced-data delivery with provenance, and the standing "this is not approval" line.
8. Receipts (queued, delivered, acknowledged).
9. A receiver budget at least as strict as the native one.
10. A `SECURITY.md` of limits, and a license.

**Labels and clearance** are the differentiator and the reason this project is worth doing if the
use is across a public and a private repository. They are also the largest piece of work. A first
release that omits them must say so loudly and be used only among repositories of one label
(`DESIGN.md` section 7).

## 10. What would change this list

- A tool surveyed here adds labels or clearance, which would move them from a differentiator to a
  market feature.
- Claude Code adds rooms or promotes channels out of research preview, which would change the
  documented-features-only constraint (TS5) and probably make several of these tools redundant.
- Issue #92060 (folder scope crossed through `SendMessage`) is fixed natively, which would reduce
  the urgency of C4 for the same-machine case but not for a room that carries content to many
  members.
- A broader survey including closed products moves the counts.
