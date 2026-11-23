# klatch client libraries

New here? [INTEGRATION.md](../INTEGRATION.md) is the guide, with two runnable examples in `clients/examples/`.

Thin clients over the wire protocol (`PROTOCOL.md`, version 1): they start `klatch serve --stdio` and speak newline-delimited
JSON to it, or connect to the `klatch-server daemon` over its Unix socket. Over stdio, who a client is comes from the
environment the process is started with (a name, or a key bundle), never from a request. They hold no policy: the gateway
decides, and a refusal comes back as an error carrying the rule.

## Over the daemon's socket

`klatch-server daemon --socket PATH` (#102) serves the same protocol to local connections. Each library connects with a path
and an identity, which it sends as the daemon's first request, `identify`; the daemon answered `hello` first, so the library
reads it as over stdio, then sends `identify`, and `member` and `attribution` are what the daemon answered.

| Identity | Python | TypeScript | Rust | Member |
|---|---|---|---|---|
| a typed name | `Client.connect_socket(path, name="alice")` | `Client.connectSocket(path, { name: "alice" })` | `Client::connect_socket(&path, &Identity::name("alice"))` | `alice`, unattributed |
| a credential | `member="w-1", token=...` | `{ member: "w-1", token }` | `Identity::credential("w-1", &token)` | `w-1`, verified |
| none | `Client.connect_socket(path)` | `Client.connectSocket(path)` | `Identity::none()` | `unmapped-process`, unattributed |

A refused identity (a bad name, a token that is not live, another member's name with a token) ends the connection with an
error whose code is `identity.refused`; its message never repeats the name or the token. A path nobody listens on is the
socket's own error. `close` shuts the socket and waits for the daemon to end the session; it answers 0. Only the token goes
over the socket, never the key bundle file. The Rust connect is the `client-socket` feature of `klatch` (off by default and
in no default feature, so the `klatch` binary contains no socket code); `Client::over` takes any reader and writer.

The conformance scenario of each library runs twice, over stdio and, unchanged, over the daemon (Python and TypeScript when
`klatch-server` is built, `KLATCH_SERVER_BIN` or `proto/target/debug/klatch-server`; Rust in
`proto/klatch-server/tests/client_socket.rs`, which includes `proto/klatch/tests/client_scenario`).

## Over HTTP and server-sent events

`klatch-server http --listen 127.0.0.1:PORT` (#1059) serves the protocol over HTTP, loopback only, and every caller needs a
worker credential the human issued. Python connects with `Client.connect_http("http://127.0.0.1:PORT", member="w-1",
token=...)` and TypeScript with `Client.connectHttp(url, { member: "w-1", token })` (#1114). The library opens `GET
/v1/stream`, reads `X-Klatch-Session`, reads each event's `data:` as a frame, and sends each request as `POST /v1/frames`
with that header; the response arrives on the stream. A refused credential is an error with code `identity.refused`. A url
that is not plain `http` to a loopback address is refused before anything is sent. If the stream drops, the library reopens it
and resubscribes every subscribed room from the last `seq` seen, so messages resume without a gap or a repeat. Rust connects
with `Client::connect_http(url, member, token)` (feature `client-http` of `klatch`, off by default and in no default feature, so
the `klatch` binary has no network socket type; one file, `proto/klatch/src/client/http.rs`, names `TcpStream`). Its conformance
run is `cargo test -p klatch-server --test client_http` (#1122).

| Language | Where | Dependencies | Conformance test |
|---|---|---|---|
| Python 3 | `clients/python/klatch_client.py` | standard library | `cd clients/python && KLATCH_BIN=... python3 -m unittest -v` |
| TypeScript | `clients/typescript/klatch.ts` | Node 22 standard library (types stripped by Node, no build step) | `cd clients/typescript && KLATCH_BIN=... node --experimental-strip-types --test conformance.test.ts` |
| Rust | `klatch::client` in `proto/klatch` (feature `client`) | `klatch::protocol` | `cargo test -p klatch --test client` |

`KLATCH_BIN` is the klatch binary (default: the workspace's debug build). Each test starts its own throwaway store.

## What every client does

Connect and read `hello` (member, attribution, clearance); `request` (any method, a result or an error with its code and any
refusal frames); `post` (with optional label, kind and payload), `read`, `history` (look back with optional `last` and `since`, no cursor moved, returning the messages and `more`), `claim`, `release`, `claims`, `who`; `subscribe` with
resume from a sequence number, then messages, `caught_up` and live pushes, and `ack`; close, which ends the session by closing
its input. A frame type or field the client does not know is ignored; a frame that is not `"v":1` is a protocol error.

## Versioned with the protocol

Each library exports `PROTOCOL_VERSION` (1) and refuses any other. The conformance scenario is the same in all three
(hello, every protocol fixture line decoded and unknown things ignored, a post read by another member with a handoff's kind
and payload kept, a history look-back that moves no cursor and pages with `since`, replay then catch-up then push then resume, a claim conflict and release, a refusal that names its rule and
never repeats the text, bad requests that fail without ending the session, and a clean close). A change to the protocol is a
change to the fixtures in `proto/klatch/fixtures/protocol/`, which all three read.

## Over a WebSocket

`GET /v1/ws` (#112, experimental) on `klatch-server http` (loopback, `ws://`) and `klatch-server https` (mutual TLS, `wss://`)
carries the same session as `GET /v1/stream` plus `POST /v1/frames` over one connection: the server sends `hello` first, a
frame line (the JSON of `PROTOCOL.md`, without its newline) is one text message each way, a `response` arrives as a message
as it does on the other transports, and a new connection resumes with `subscribe` and `from_seq` set to the last `seq` seen.
Any WebSocket library works; the client libraries here have no WebSocket transport (not built).

```
GET /v1/ws HTTP/1.1
Upgrade: websocket
Connection: Upgrade
Sec-WebSocket-Version: 13
Sec-WebSocket-Key: <16 random bytes, base64>
Authorization: Bearer TOKEN
X-Klatch-Member: MEMBER
```

The credential is checked at the upgrade (any other result is the one `401`, and no `Sec-WebSocket-` header). Send no `Origin`
header: a non-browser client has none to send, and a request that carries one is refused with `403` unless the operator
started the server with `--ws-allow-origin` naming exactly that origin. A browser cannot set `Authorization` on a WebSocket,
so a page cannot connect without a shim that holds the credential. Over TLS the client also presents its certificate, which
only gates the port. Frames must be masked (libraries do this), a message is at most 1 MiB, binary messages are refused,
and the client must answer pings (libraries do this too): a connection that sends nothing for 60 seconds is closed with
1001. A text message may hold several frame lines, which are several frames.
