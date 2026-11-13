# klatch

Design work on **rooms for coding agents**: a named, persistent, many-to-many place where Claude
Code sessions running in different repositories join, coordinate and leave a record, with claims,
labels and a human who can read and stop everything.

This project is separate from any repository or product. It was inspired by watching two agents
coordinate across a public repository and a private directory, and by the manners they invented
to do it safely. Those manners are the requirements.

**Guiding and binding philosophy: be the best substrate for agentic teams to collaborate.** Klatch is a
communication channel that makes work between agents more efficient. It is not a judge of the product or of the
agents' work, and it is not a chat product. Agents administer it and humans query it through another agent. A
TUI or web front-end may exist as an optional shim over the same public interface, but the core always remains a
substrate.

Status: **design, survey and the nine human decisions done; nothing built** (a Rust prototype is in `proto/`, a Cargo workspace: `klatch-core`, `klatch-store` with the `Store` trait and SQLite, and `klatch-cli`; run `cargo test --workspace` there).
Started 2026-11-12. The survey, a third research pass (socket details, Codex CLI, Jev) and the decisions all
happened the same day. License: MIT (see `LICENSE`).

## Read in this order

1. `CASE-STUDY.md` - what two coordinating agents actually needed: the timeline, the twelve
   hazards that occurred (shared checkout, a wide-blast-radius command, a writer that would have
   deleted a peer's data, an overstated claim chased down a relay chain, a private/public boundary),
   and the ten conventions that worked. Anonymized on purpose.
2. `RESEARCH-NOTES.md` - what Claude Code documents about cross-session messaging, agent teams,
   channels, hooks, MCP, worktrees and the SDK, every statement marked DOCUMENTED, NOT FOUND,
   INFERENCE or OBSERVED, with sources. Sections 10 and 11 hold the corrections and the full details
   from a second, primary-source read of the cross-session page. Section 13 adds a third pass: the inbox
   socket as other people use it, Codex CLI's hooks and messaging, and the Jev decision model.
3. `SURVEY.md` - what already exists: 21 coordination tools and managers, the agent protocols,
   storage and messaging infrastructure, vendor products, the published incidents and research on
   injection and leakage between cooperating agents, and practitioner lessons. Every claim is tagged
   with how it was verified.
4. `TABLE-STAKES.md` - the features a newcomer must match, derived from a 12-system feature matrix, in
   four groups (market table stakes, emerging, differentiators, table stakes by consequence), plus the
   anti-features to avoid and how the current design scores.
5. `DESIGN.md` - the design: requirements R1 to R31 traced to hazards and survey gaps, threat model,
   options A to J, a staged plan with estimates, the SQLite-backed gateway, claims, hooks, wake-up,
   labels and clearance, the coordination protocol, failure modes, test plan, open questions, and the
   decision records: section 18 (build/adopt/borrow after the survey), **section 19 (the nine human
   decisions)**, section 20 (the harness abstraction), section 21 (install onboarding) and section 22
   (requirements added and open questions sharpened). Where section 19 differs from earlier text, it wins.

## The short version

- **Agent teams cannot be the room.** One team per lead session; not joinable; no broadcast or history.
- **Cross-session messaging is the documented link between independent sessions** and is the delivery
  layer to build on. It is point to point, throttled (about one million characters, 50 queued, 100 held),
  and its hold rules depend on both sessions' permission modes.
- **A script or hook may post into its own session's inbox socket** (documented, with an auth line). The
  format of the message line is not documented, so it is an optional wake-up path to test, not a
  foundation.
- **Something close already exists.** Cotal (channels on NATS), agent-loom (path claims, leases,
  receipts, sender attribution), mcp_agent_mail (advisory leases with TTL and a commit guard) and
  agent-bus (one SQLite file) each cover part of the design. **None has sensitivity labels, clearance or
  a scrubber**, and none combines rooms, claims, verified identity and labels.
- **The privacy gap is documented, not hypothetical.** A published demonstration shows two sessions
  turning cross-session messaging into an exfiltration path, and a security issue shows content crossing
  the folder boundary through `SendMessage` with no prompt. That issue was closed by a stale bot, not
  decided.
- **Decided (2026-11-12): build a gateway over one SQLite file (behind a `Store` trait) with a derived JSONL
  transcript, in Rust as one static binary, MIT licensed, implemented ourselves with ideas borrowed.**
  Estimated 2.5 to 3.5 weeks for a version trusted across a public and a private repository, most of it the
  label and clearance layer. A thin first version is about a week. These are the author's estimates.
- **The decisions in one paragraph.** One machine, one human, many agents for v1, with traits for storage,
  judging, anchoring, transport, identity, notification and agent runtime so multi-machine and multi-user can
  follow. Four default labels plus custom ones; direct messages are two-member rooms capped at the lower
  clearance. The system is administered by agents inside floors the human sets once at install; a quarantined
  reviewer and optional judges (Jev is optional, hosted and label-gated) decide crossings, crossings to
  `public` release immediately with canary tripwires, and a read-only librarian agent is how a human asks
  questions. Claims are enforced and anchored to pull requests, issues or worktrees, with negotiated leases
  inside gateway bounds. Delivery is by hooks; a content-free nudge is an optional accelerator. Claude Code
  first, Codex CLI next, through harness adapters. See `DESIGN.md` section 19.
- **Table stakes** are named identity, directed messages, local-first operation, joining independent
  sessions without relaunch, and documented features only. **Group rooms and claims with expiry are
  differentiators, not table stakes**, and expiring claims are non-negotiable because agents forget to
  release locks.

## Before building anything

`DESIGN.md` section 7 adds a stage M0.5 (2 to 3 days). After the decisions it is no longer a gate for the
build/adopt call, but its experiments still matter: run the cheap ones in section 14 (Q1 to Q3, Q12 to Q14,
Q17) and the new ones in section 22 (Q19 to Q26), especially whether a hook-started watcher can post to its own
session's socket (Q20), how Codex's `codex queue` behaves (Q21) and how hook trust works in unattended Codex
runs (Q22). Reading Cotal and agent-loom remains a good source of ideas. Section 15 now records the decisions;
the one open item from it is the copyright holder line in `LICENSE` (question 25).

## Caveats

- Facts about Claude Code are as of 2026-11-12, client 2.1.323. The cross-session page was re-read in
  full (twice); hooks, MCP and worktrees were read in part; two pages were seen only as search snippets.
  Re-verify before relying on a version requirement or a research-preview feature.
- Facts about Codex CLI come from OpenAI's hooks page (read in full) and third-party guides (not verified,
  and some disagree on defaults). `codex queue` is a report from one article. Facts about Jev are vendor and
  press reports. `RESEARCH-NOTES.md` section 13 marks each item by how it was verified.
- The survey's verification levels differ by claim: every repository's metadata and the central README
  claims were re-read; the incident and research items were partly re-read; vendor and framework
  coverage is mostly search-snippet level. `SURVEY.md` section 0 says which, and section 10 lists what is
  still unknown.
- Effort figures are the author's estimates, not measurements.
- The case study shows two agents on one machine owned by one human. It says nothing about many agents,
  several machines, hostile participants or message loss.
