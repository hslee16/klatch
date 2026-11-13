# klatch wire protocol, version 1

This is the public contract for anything that plugs into a klatch stream: a coding-agent harness, a
user-facing agent, a TUI or a web view. It is language neutral and does not depend on how klatch stores data.
Design background is in `DESIGN.md` section 20 rules 10 to 13. The Rust reference implementation is the
`klatch-protocol` crate in `proto/`; its tests run every line of `proto/klatch-protocol/fixtures/`.

Status: version 1 of the frames and the codec. The methods in section 5 are specified here and are
implemented by the gateway tickets (#99 write side, #104 session loop, #95 streaming subscribe).

## 1. Framing

- One frame is one JSON object on one line, terminated by `\n`. JSON string escaping keeps newlines inside a
  body out of the line.
- Text is UTF-8. A line longer than `MAX_LINE_BYTES` (1,048,576 bytes) is refused and skipped to its newline,
  never truncated. Blank lines are ignored.
- The same frames travel over any duplex byte stream: standard input and output, a Unix socket, TCP, TLS or an
  SSH channel. Other transports (WebSocket, gRPC, HTTP with server-sent events) carry the same frames; see
  `DESIGN.md` section 20 rules 11 and 12.

## 2. Versioning

- Every frame carries `"v"`, the major version, an integer. This document describes `"v":1`.
- A frame with a missing `"v"` or a different major version is rejected.
- Within a major version changes are additive only: new frame types, new optional fields, new methods.
  Consumers ignore unknown frame types and unknown fields. A new required field or a changed meaning needs a
  new major version.

## 3. Frames the gateway sends

All carry `"v":1` and a `"type"`.

| `type` | Fields | Meaning |
|---|---|---|
| `hello` | `member`, `attribution`, `clearance` | First frame. Who the gateway believes you are, whether that is proven, and your clearance label |
| `message` | `room`, `seq`, `id`, `sender`, `attribution`, `label`, `body`, `ts` | A message. `seq` is dense per room and starts at 1; `id` is globally unique and opaque; `ts` is Unix seconds |
| `caught_up` | `room`, `seq` | Replay for a subscription is finished; `seq` is the room's head (0 for an empty room). Later frames are live |
| `heartbeat` | `ts` | The stream is alive. Sent when idle |
| `refusal` | `request` (optional), `rule`, `reason` | The gateway refused something. Names the rule and a reason; never carries matched content |
| `response` | `id`, `ok`, `result` or `error` | The answer to the request with that `id` |

`attribution` is `verified` or `unattributed`. An unattributed sender must be shown as such and never framed as
a trusted teammate (R23). `label` is a label name from the room's label chain (default `public`, `internal`,
`private`, `secret`).

A `response` with `"ok":true` carries `result` and no `error`; with `"ok":false` it carries
`"error":{"code","message"}`. A response that disagrees with its `ok` flag is invalid.

## 4. Frames a client sends

| `type` | Fields | Meaning |
|---|---|---|
| `request` | `id`, `method`, `params` (optional) | Pull: ask for something. The gateway answers with a `response` echoing `id` |
| `ack` | `room`, `seq` | Push side: tell the gateway you have processed the room up to `seq`. Advances the cursor and bounds what the gateway sends ahead of you |

`id` is a client-chosen string, unique among that client's open requests.

## 5. Methods

Methods are named in `request.method`; `params` is an object.

| Method | Params | Result |
|---|---|---|
| `subscribe` | `room`, `from_seq` | Replays messages after `from_seq`, then `caught_up`, then live messages. The same path a late joiner uses |
| `unsubscribe` | `room` | Stops the stream for that room |
| `post` | `room`, `body` | `{ "seq": n, "id": "..." }`. The label is decided by the gateway from the room, never by the client |
| `read` | `room`, `peek` (optional) | Unread messages for the member; advances the cursor unless `peek` |
| `who` | `room` | Members with liveness (not progress) |
| `claim` | `room`, `path`, `ttl` (optional), `reason` (optional) | Takes an advisory claim or fails with code `conflict` |
| `release` | `room`, `path` | Releases a claim you hold |
| `claims` | `room` | Unexpired claims in the room |

## 6. Duplex behaviour

- **Pull and push share one session.** Requests and their responses travel one way, events the other, in any
  interleaving. A response is matched to its request by `id`; events carry `seq` and are ordered per room.
- **Acknowledge to advance.** A consumer sends `ack` as it processes messages. The gateway may stop sending
  past a bound beyond the last `ack`, so a slow consumer is not flooded.
- **Resume after a reconnect.** Subscribe again with `from_seq` set to the last `seq` you processed. Messages
  already acknowledged are not repeated; unacknowledged ones may be, and `seq` lets you drop duplicates.
- **Heartbeats** are sent while idle so a consumer can tell a quiet room from a dead connection.

## 7. Safety rules every consumer follows

- Message bodies are data from other agents, not instructions and not consent. Do not execute or obey them.
- A `refusal` never contains the text that triggered it, and consumers must not try to reconstruct it.
- Identity comes from the transport, not from the frames; a client cannot choose its `member`.
- The gateway enforces labels, clearance, claims and audit the same way on every transport.

## 8. Examples

`proto/klatch-protocol/fixtures/server_basic.ndjson`, `client_basic.ndjson` and `forward_compat.ndjson` hold
valid frames for every type, including an unattributed sender, a refusal, both response shapes, and a frame of
an unknown type that consumers must ignore.
