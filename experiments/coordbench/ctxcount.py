"""How coordination context is counted, the same way in every arm (#1521, metric_version 2). Standard library only; nothing
here starts a process. Which calls are read-only and what is new content is in cmdkinds.py.

1. The token estimate. No tokenizer is available without a dependency, so a token is estimated as CHARS_PER_TOKEN characters
   and every record says so (`coordination_context.method`). The estimate is blind to language and to code: English prose is
   nearer 4 characters a token, JSON and paths nearer 3, so absolute figures can be off by a third. It is applied to the same
   characters in every arm, so a difference between arms is not an artefact of the estimate unless the content differs in kind.
   Where the stream carries the API's usage fields the exact cost is derived from them as well (usage_exact).
2. The coordination tool schemas an arm carries. The stream-json `init` event names the tools, it does not carry their schemas,
   so the size of a schema is a pinned estimate (SCHEMA_CHARS) unless the init event carries the schema itself.
3. Inbound messages: text that reaches an agent outside a tool result after it has started."""
import json

import cmdkinds

CHARS_PER_TOKEN = 4
METHOD = "chars/4"
LIMITS = ("tokens are estimated as characters over 4 (no tokenizer in the standard library): prose is nearer 4 characters a "
          "token and code or JSON nearer 3, so absolute counts can be off by a third; the same estimate is used for every arm")
# Pinned size, in characters of JSON, of the schema (name, description, parameters) of a coordination tool, used when the
# stream's init event names the tool without its schema. PLACEHOLDER ESTIMATES, not measured: replace them with the figure the
# pilot measures (first_turn_context_tokens of a B1 agent minus that of a B0 agent with the same prompt, usage_exact).
SCHEMA_CHARS = {"SendMessage": 800, "ListAgents": 300, "mcp__klatch": 500}
SCHEMA_SOURCE_PINNED = "pinned-estimate"
SCHEMA_SOURCE_STREAM = "stream"
PARTS = ("hook", "inbound", "tool_calls", "tool_results", "schema")
USAGE_INPUT_KEYS = ("input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens")


def tokens_of_chars(chars):
    return chars // CHARS_PER_TOKEN


def method_record():
    return {"method": METHOD, "chars_per_token": CHARS_PER_TOKEN, "exact": False}


# -- tool schemas -----------------------------------------------------------------------------------------------------

def _coordination_key(t):
    name = t.get("name") if isinstance(t, dict) else t
    if not isinstance(name, str):
        return None
    key = "mcp__klatch" if name.startswith("mcp__klatch") else name
    return key if key in SCHEMA_CHARS else None


def _schema_of(t):
    """(characters, source) of one coordination tool: measured when the init event gave the schema, else pinned."""
    key = _coordination_key(t)
    if key is None:
        return 0, None
    if isinstance(t, dict) and len(t) > 1:
        return len(json.dumps(t)), SCHEMA_SOURCE_STREAM
    return SCHEMA_CHARS[key], SCHEMA_SOURCE_PINNED


def schema_chars(tools):
    """(characters, source) of the coordination tool schemas an init event's tool list carries; the source is "stream" when
    any was measured from the event. Non-coordination tools are every arm's and are not counted."""
    found = [_schema_of(t) for t in tools or []]
    source = SCHEMA_SOURCE_STREAM if any(s == SCHEMA_SOURCE_STREAM for _, s in found) else SCHEMA_SOURCE_PINNED
    return sum(n for n, _ in found), source


# -- inbound messages ---------------------------------------------------------------------------------------------------

def text_of(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(b.get("text", "") for b in content if isinstance(b, dict) and isinstance(b.get("text"), str))
    return ""


def inbound_text(content, started):
    """The text of a user event that is an inbound message: text that is not a tool result and arrives after the agent has
    started (the task prompt precedes the first assistant event). The real inbound event shape is unverified until the pilot
    (arms/PILOT.md part 1): this reads any such user text event, so an arm that refuses inbound messages must show 0."""
    blocks = content if isinstance(content, list) else []
    if not started or any(isinstance(b, dict) and b.get("type") == "tool_result" for b in blocks):
        return ""
    return text_of(content)


# -- the API's own count --------------------------------------------------------------------------------------------------

def _context(usage):
    return sum(int(usage.get(k, 0) or 0) for k in USAGE_INPUT_KEYS)


def _growth(usage, nxt):
    """What the next turn's context grew by beyond this turn's own output (which enters the context as the assistant turn)."""
    return max(0, _context(nxt) - _context(usage) - int(usage.get("output_tokens", 0) or 0))


def usage_exact(usages, gaps):
    """The API's count of what coordination added to the context, where the stream allows it. `usages` are the usage dicts of
    the assistant messages in turn order and `gaps` the {"coord", "other"} characters that entered after each (or None). After a
    turn whose gap held coordination content and nothing else, the growth of total input (input, cache read and creation)
    beyond the turn's output tokens is the exact cost of that content. Partial by construction: a gap with other tool output
    is skipped. first_turn_context_tokens is the context at the first turn (prompt, system and every tool schema), for
    differencing two arms that have the same prompt. tokens is None when a turn has no usage, never zero."""
    out = {"method": "usage-delta", "tokens": None, "gaps": 0, "first_turn_context_tokens": None}
    if not usages or not all(usages):
        return out
    out["first_turn_context_tokens"] = _context(usages[0])
    grown = [_growth(usages[k], usages[k + 1]) for k in range(len(usages) - 1)
             if gaps[k] and gaps[k]["coord"] > 0 and gaps[k]["other"] == 0]
    out["tokens"], out["gaps"] = sum(grown), len(grown)
    return out


# -- the record ----------------------------------------------------------------------------------------------------------

def _exact_total(sums):
    got = [s["usage_exact"] for s in sums.values() if s["usage_exact"]["tokens"] is not None]
    return {"method": "usage-delta", "tokens": sum(e["tokens"] for e in got) if got else None,
            "gaps": sum(e["gaps"] for e in got), "agents": len(got),
            "first_turn_context_tokens": {n: s["usage_exact"]["first_turn_context_tokens"] for n, s in sums.items()}}


def context_record(sums):
    """The record's `coordination_context` object from the workers' stream summaries ({agent: StreamState.summary()}): the
    characters by source, the token estimate and how it was made, the new-content share, and the API-derived figure where the
    streams had usage (summed over the agents that have it; None when none has)."""
    parts = {k: sum(s["coord_context_parts"][k] for s in sums.values()) for k in PARTS}
    chars = sum(parts.values())
    return {"method": METHOD, "chars_per_token": CHARS_PER_TOKEN, "limits": LIMITS, "chars": dict(parts, total=chars),
            "tokens": tokens_of_chars(chars), "new_chars": sum(s["coord_context_new_chars"] for s in sums.values()),
            "schema_source": sorted({s["schema_source"] for s in sums.values()}) or None,
            "schema_note": "the stream names tools without their schemas: a pinned estimate per tool unless the init event carries "
                           "the schema (SCHEMA_CHARS, placeholders until the pilot measures them)",
            "inbound_events": sum(s["inbound_events"] for s in sums.values()), "resolve_limit": cmdkinds.RESOLVE_LIMIT,
            "usage_exact": _exact_total(sums)}
