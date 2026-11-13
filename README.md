# klatch

Design work on **rooms for coding agents**: a named, persistent, many-to-many place where Claude
Code sessions running in different repositories join, coordinate and leave a record, with claims,
labels and a human who can read and stop everything.

This project is separate from any repository or product. It was inspired by watching two agents
coordinate across a public repository and a private directory, and by the manners they invented
to do it safely. Those manners are the requirements.

Status: **design and survey done; nothing built.** Started 2026-11-12. The survey ran the same day.

## Read in this order

1. `CASE-STUDY.md` - what two coordinating agents actually needed: the timeline, the twelve
   hazards that occurred (shared checkout, a wide-blast-radius command, a writer that would have
   deleted a peer's data, an overstated claim chased down a relay chain, a private/public boundary),
   and the ten conventions that worked. Anonymized on purpose.
2. `RESEARCH-NOTES.md` - what Claude Code documents about cross-session messaging, agent teams,
   channels, hooks, MCP, worktrees and the SDK, every statement marked DOCUMENTED, NOT FOUND,
   INFERENCE or OBSERVED, with sources. Sections 10 and 11 hold the corrections and the full details
   from a second, primary-source read of the cross-session page.
3. `SURVEY.md` - what already exists: 21 coordination tools and managers, the agent protocols,
   storage and messaging infrastructure, vendor products, the published incidents and research on
   injection and leakage between cooperating agents, and practitioner lessons. Every claim is tagged
   with how it was verified.
4. `TABLE-STAKES.md` - the features a newcomer must match, derived from a 12-system feature matrix, in
   four groups (market table stakes, emerging, differentiators, table stakes by consequence), plus the
   anti-features to avoid and how the current design scores.
5. `DESIGN.md` - the design: requirements R1 to R27 traced to hazards and survey gaps, threat model,
   options A to J, a staged plan with estimates, the SQLite-backed gateway, claims, hooks, wake-up,
   labels and clearance, the coordination protocol, failure modes, test plan, open questions, decisions
   and, in section 18, the build/adopt/borrow decision record.

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
- **Recommendation (provisional): build a gateway over one SQLite file with a derived JSONL transcript,
  borrow designs, and read Cotal and agent-loom in full before writing code.** Estimated 2.5 to 3.5
  weeks for a version trusted across a public and a private repository, most of it the label and
  clearance layer. A thin first version is about a week.
- **Table stakes** are named identity, directed messages, local-first operation, joining independent
  sessions without relaunch, and documented features only. **Group rooms and claims with expiry are
  differentiators, not table stakes**, and expiring claims are non-negotiable because agents forget to
  release locks.

## Before building anything

`DESIGN.md` section 7 adds a stage M0.5 (2 to 3 days): read Cotal and agent-loom in full, look at
agent-bus's schema and mcp_agent_mail's reservation tables, run the cheap experiments in section 14
(Q1 to Q3, Q12 to Q14, Q17), and write the decision record. Section 15 lists the decisions the human
needs to make, including the license.

## Caveats

- Facts about Claude Code are as of 2026-11-12, client 2.1.323. The cross-session page was re-read in
  full; hooks, MCP and worktrees were read in part; two pages were seen only as search snippets.
  Re-verify before relying on a version requirement or a research-preview feature.
- The survey's verification levels differ by claim: every repository's metadata and the central README
  claims were re-read; the incident and research items were partly re-read; vendor and framework
  coverage is mostly search-snippet level. `SURVEY.md` section 0 says which, and section 10 lists what is
  still unknown.
- Effort figures are the author's estimates, not measurements.
- The case study shows two agents on one machine owned by one human. It says nothing about many agents,
  several machines, hostile participants or message loss.
