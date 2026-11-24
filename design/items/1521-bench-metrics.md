# 1521: coordbench metrics mean the same thing in every arm

Tickets #1521 (coordination context counted asymmetrically), #1522 (S1's primary measured model ability) and #1524 (idle turns
and B-arm metrics defined asymmetrically), all from the independent review of #1461.

## Problem

- The same 458-character answer counted 0 as a B1 inbound message and 458 as a K2 hook digest, B1's tool schemas were never
  counted, and the estimate (characters over 4) was written nowhere but a note.
- Every klatch read, claims, asks and who turn was idle; B1's TASKS.md read, git pull and ListAgents were not (3 of 3 K turns
  idle against 1 of 5 equivalent B1 turns). SPEC 5's "no new content" was never checked.
- `claim_conflicts` was null for B arms, board edits were not messages, a klatch call through a shell variable was missed.
- S1's primary counted the eight modules, one owner each, so every arm failed the tokenizer unit and the coordination layer
  was almost invisible in the headline.

## Decision

- One counting path (`ctxcount.py`, `streams.py`): five sources (hook output, inbound message events, coordination call
  inputs, their results, init tool schemas), the estimate and its limits written in every record (`coordination_context`), and
  an exact figure from the API usage fields where a gap held only coordination content. No tokenizer dependency.
- One idle definition with per-arm read-only equivalents and a new-content check (no-news phrases for calls that print only
  what is new; a diff against the agent's last result for calls that print the whole state).
- B1 claim conflicts from the board's git history (`board.py`); board edits are messages; klatch calls through a variable,
  alias or function defined in the same command are resolved (the limit is stated).
- S1's primary is `integration_ok`, from the checker's structural checks, with `units_completed` secondary.
- `metric_version: 2` in every new record; `report.py` splits mixed versions into separate groups and flags comparisons across
  versions (`metricver.py`).

## Not decided here (left to others)

- The 12. example in the tokenizer docstring is a fixture change (it changes the fixture hash): the fixture builder owns it.
- Schema sizes are placeholders; the pilot measures them (first_turn_context_tokens of B1 minus B0).
- The real B1 inbound event shape is unverified (PILOT part 1); the parser reads any user text event after the first assistant
  event as inbound.
- Whether the tallykit checker's marker scan should skip TASKS.md (#1461 I1) is the checker owner's call.
- S3's `idle_turns` (chain.py) keeps its own blocked-turn definition.
