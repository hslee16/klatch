# klatch client libraries

Thin clients over the wire protocol (`PROTOCOL.md`, version 1): they start `klatch serve --stdio` and speak newline-delimited
JSON to it. Who a client is comes from the environment the process is started with (a name, or a key bundle), never from a
request. They hold no policy: the gateway decides, and a refusal comes back as an error carrying the rule.

| Language | Where | Dependencies | Conformance test |
|---|---|---|---|
| Python 3 | `clients/python/klatch_client.py` | standard library | `cd clients/python && KLATCH_BIN=... python3 -m unittest -v` |
| TypeScript | `clients/typescript/klatch.ts` | Node 22 standard library (types stripped by Node, no build step) | `cd clients/typescript && KLATCH_BIN=... node --experimental-strip-types --test conformance.test.ts` |
| Rust | `proto/klatch-client` | `klatch-protocol` | `cargo test -p klatch --test client` |

`KLATCH_BIN` is the klatch binary (default: the workspace's debug build). Each test starts its own throwaway store.

## What every client does

Connect and read `hello` (member, attribution, clearance); `request` (any method, a result or an error with its code and any
refusal frames); `post` (with optional label, kind and payload), `read`, `claim`, `release`, `claims`, `who`; `subscribe` with
resume from a sequence number, then messages, `caught_up` and live pushes, and `ack`; close, which ends the session by closing
its input. A frame type or field the client does not know is ignored; a frame that is not `"v":1` is a protocol error.

## Versioned with the protocol

Each library exports `PROTOCOL_VERSION` (1) and refuses any other. The conformance scenario is the same in all three
(hello, every protocol fixture line decoded and unknown things ignored, a post read by another member with a handoff's kind
and payload kept, replay then catch-up then push then resume, a claim conflict and release, a refusal that names its rule and
never repeats the text, bad requests that fail without ending the session, and a clean close). A change to the protocol is a
change to the fixtures in `proto/klatch-protocol/fixtures/`, which all three read.
