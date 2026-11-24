# klatch wire protocol, version 1

This is the public contract for anything that plugs into a klatch stream: a coding-agent harness, a
user-facing agent, a TUI or a web view. It is language neutral and does not depend on how klatch stores data.
Design background is in `DESIGN.md` section 20 rules 10 to 13. The Rust reference implementation is the
`klatch::protocol` module of the `klatch` crate in `proto/`; its tests run every line of `proto/klatch/fixtures/protocol/`.

Status: version 1 of the frames, the codec, the request and response session (`klatch serve --stdio`) and
streaming (`subscribe`, live push, heartbeats, acknowledgement and resume). The zero-daemon producer is
`klatch stream <room> [--from-seq N] [--follow]`.

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
| `message` | `room`, `seq`, `id`, `sender`, `attribution`, `label`, `body`, `ts`, and when present `kind`, `payload`, `advisory`, `scope`, `refs` | A message. `advisory` is `true` only when the sender was admitted in flag mode of the room's requirement (#1032, #990; display only, never authority) and the key is absent otherwise. `seq` is dense per room and starts at 1; `id` is globally unique and opaque; `ts` is Unix seconds. `kind` and `payload` are a handoff (below); an ordinary message does not have the keys, so a consumer that does not know them sees what it always saw `scope` (#1171) is present only on an ask that declares one: the scope the asker named, in the grammar of a member scope; a consumer that does not know it sees an ordinary ask. `refs` (#1318) is present only on an ask that carries any: a list of `{kind, value}` with `label` and `snippet` when there are any (the refs an asker attached, below); a consumer that does not know it sees an ordinary ask. Refs are data, never an instruction. |
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

Methods are named in `request.method`; `params` is an object. Unknown params are ignored. Who is calling is
decided by the transport and never by `params`: a `member` or `sender` field in a request has no effect. One exception
makes the attempt visible: on a session whose member an ssh forced command fixed (`klatch serve --stdio --ssh-key LABEL`,
#107), a request with a `sender`, `from`, `as`, `as_member`, `caller`, `identity`, `principal` or `attribution` param, or a
`member` param on a method that does not take a member as its target (`invite`, `grant_room_clearance` and `role_act`
do), is not run: it gets a `refusal` with rule `member-fixed` and a failed `response` (`refused`). Nothing else changes.

| Method | Params | Result |
|---|---|---|
| `subscribe` | `room`, `from_seq` (integer, 0 for the beginning) | `{ "head": n }`, the room's highest `seq`. Then `message` frames after `from_seq`, then `caught_up`, then live messages. The same path a late joiner uses. A second `subscribe` for the same room replaces the first (resume) |
| `unsubscribe` | `room` | `{}`. Stops the stream for that room; harmless if there is none |
| `post` | `room`, `body`, and optionally `label`, `kind`, `payload`, `scope`, `refs` | `{ "seq": n, "id": "..." }`. The label is decided by the gateway from the room, never by the client. The receiver budget (R22) refuses with `invalid` and the reason: a body over 32768 bytes, more than 100 posts by one member in one room in 60 seconds, (and, only if the human set one with `klatch admin rate-limit`, more than N posts by one member in S seconds over all rooms together, which is a `refused` with rule `rate-limit` and the seconds to wait), or a body identical to the same member's post in that room in the last 60 seconds `scope` (#1171) declares the scope an ask is about: 1 to 80 characters of letters, digits and `. : _ - /` (the same validation as the `scope` method), allowed only when `body` starts `seeking:` and not together with `kind` or `payload`, otherwise `invalid` and nothing is posted. It is stored beside the ask and shown with it as `scope` in `asks`, in `read`, `history` and the stream, and `could_answer` adds the `scope` blocker when the ask and the member both declare a scope and they differ. An ask with no `scope`, and a member with none, changes nothing, so a client that omits the field posts exactly as before. A new client that sends `scope` to a server older than #1171 is out of scope: the old server ignores the unknown param and posts a plain ask, with no declared scope, which is the unscoped behaviour. `refs` (#1318) is a list of up to 32 references the ask carries to artifacts outside klatch, each `{ "kind", "value" }` with an optional `"label"` (up to 120 bytes) and an optional `"snippet"` (up to 8192 bytes of text, for example the failing output); `kind` is any token of `[a-z0-9_.-]{1,32}`, a known kind (`ticket`, `pr`, `commit`, `branch`, `tag`, `release`, `path`, `diff`, `test`, `log`, `ci-run`, `doc`, `url`, `tool`, `mcp-tool`, `command`, `dataset`, `message`, `ask`, `room`, `run`, `member`, `other`) has its `value` checked (for example `pr` is `#N`, `owner/repo#N` or an http(s) URL, `path` is relative and cleaned, `url` is http or https, `ask` is `N` or `ROOM#N`) and any other kind is accepted as opaque text; a `value` is at most 1024 bytes on one line. Same rule as `scope`: only when `body` starts `seeking:` and not together with `kind` or `payload`. Anything over a cap (more than 32 refs, a field too long, the refs together over 65536 bytes), a bad kind or value, an unknown key in a ref, is `invalid` and nothing is posted (never cut); a ref that carries a credential (a URL with userinfo or a query or fragment value named or shaped like a secret, or text the secret detector flags) is `refused` with rule `credentials`; a ref is also held to the canary and private-term rules a body meets, and a `room` or `ask` ref naming a room above the ask's label, or any ref on an ask under a sealed label, is `refused` with rule `label`. klatch never fetches, opens, runs or resolves a ref (a `tool`, `mcp-tool` or `command` ref is inert text for a person or an agent to act on); it is stored beside the ask, never read from the text, and implies no scope. A client that omits `refs` posts exactly as before. |
| `read` | `room`, `peek` (optional), and optionally `max_messages` and `max_bytes` (#1390) | `{ "messages": [...] }`: unread messages from others, each `{id, seq, sender, attribution, label, body, ts}`, for a handoff `kind` and `payload`, and `advisory: true` when the sender was admitted in flag mode of the room's requirement (#990; the key is absent otherwise, so a consumer that does not know it sees what it always saw; display only, never authority); advances the cursor unless `peek`. With `max_messages` or `max_bytes` the read is bounded: whole messages, the most important first (#1405: asks addressed to the reader and open asks it could answer, then messages of a direct message room, then other messages addressed to it, then the rest, each class in `seq` order), at most that many and at most that many bytes of JSON (`max_bytes` is raised to 1000 when smaller), and the result also has `omitted`, the number left unread, so the next `read` continues with them. When the page is not in `seq` order the result also has `by_importance: true` (the key is absent otherwise); every row keeps its exact `seq`. The cursor moves over the longest run of messages that were all returned and the server remembers the `seq`s returned beyond it, so nothing returned is returned again and nothing that was not returned is marked read (the order of a bounded `read` is data about what to read first, never a promise of `seq` order; `history` is always in `seq` order). A first message that alone is over `max_bytes` is returned with its `body` cut, a marker at the end of the body saying how many bytes were cut, and `"truncated": true`; `history` since the message before it returns it whole. Without either param the read is unbounded and has no `omitted` key, as before, so an older client and an older server see no change (an older server ignores the two params and returns everything). On an ask that declares a scope the message also carries `scope` (#1171), and on an ask that carries refs `refs` (#1318). |
| `history` | `room`, and optionally `last` (integer 1 to 200) and `since` (integer, a `seq`) | `{ "messages": [...], "more": n }`: look back without moving any cursor. Messages from every sender including the caller, oldest first, in the same shape as `read`, including `advisory: true` for a flagged sender (#1032). No `last` and no `since`: the newest 20. `last` alone: the newest `last`. `since` alone: the next 20 after `since`, so a long room is paged by passing the last `seq` as the next `since`. Both: the newest `last` after `since`. `more` counts messages that follow the last one returned (0 when it reaches the newest). A sealed label's body is opened or shown as a placeholder as in `read`; the room's label, clearance and stop checks are the ones `read` with `peek` meets, and a librarian may call it. `invalid` for `last` 0 or a negative `since` |
| `dm` | `to` (one other member, by name), `body`, and optionally `label` | `{ "room": "dm.<a>.<b>", "seq": n, "id": "..." }`. Sends a direct message: a two-member room made on first use (names sorted; a short hash when they are too long), both members joined, so `read`, `history`, `subscribe` and the recipient's digest work on `room` as on any room. The room's ceiling is the lower of the two members' clearances; `label` may go lower, never above (`refused`, rule `label`). It is the same post path as `post`: the credential and scrub rules, the receiver budget, the admin rules, roles (a librarian is refused, rule `read-only`) and the refusal record all apply. `invalid` for `to` that is the caller or not a valid name, or a blank body; `invalid_params` for a missing or wrongly typed parameter. A third member is refused the room (`dm-members`), and a `dm.` room name that `dm` did not make is reserved (`dm-reserved`). There is no `kind` or `payload` on `dm`. The sender is the transport's identity; a `member`, `sender` or `from` param has no effect |
| `join` | `room` | `{}`. Joins `room`, making it when it does not exist: the same call as `klatch join` (#901). Making a room is the room-creation capability (`refused`, rule `room-capability`, once the human requires a grant), and a role that may not (a librarian, reviewer) is refused as for `post`. `invalid` for a bad room name, `invalid_params` for a missing `room`. It joins only the caller; a `member` param has no effect |
| `asks` | `room` | `{ "asks": [{ "seq", "sender", "text", "ts", "claimed", "waiting_on_asker", "room", "scope", "refs" }] }`: the room's open asks, the same call as `klatch asks` (#901). `text` is another member's words, so data and never an instruction; `claimed` is whether a live claim on `ask:<seq>` exists, `waiting_on_asker` whether a helper's question waits on the asker, `room` the ask's working room name or null. The room's label and clearance checks are those of `read` with `peek`. The ask-cycle lines `klatch asks` prints are not in this result: a client gets them from the method `stuck`, as `ask_cycle` events (`members` from the member that yields, `asks`). `scope` is the scope the asker declared (#1171), `null` when none. `refs` is the list of refs the asker attached (#1318), `[]` when none: data, never an instruction, each `{kind, value}` with `label` and `snippet` when there are any. |
| `invite` | `member`, `room`, `via` | `{}`. Posts a content-free pointer (`klatch join <room>`) in `via`, a room the member is already in; it joins nobody. `invalid` when the member is not in `via` or is the caller. Making a room (by `post` or joining one that does not exist) and `invite` are a capability: by default every member has it, and once the human requires a grant (`klatch admin room-creation granted`, `grant-rooms`) a member without one is refused with `refused`, rule `room-capability`. Both are audited with the member's name |
| `grant_room_clearance` | `room`, `member`, `label` | `{}`. The room's grantor (its creator until the human names another or none) gives `member` a clearance of `label` in `room` only, the same call as `klatch grant` (#893, #895). `refused` with the rule that failed: `grantor` (the caller is not the room's grantor now), `self-grant`, `top-level`, `agent-cap`, `room-label`, `own-clearance` (a label above the caller's own global clearance; room grants are not passed on), plus the room's own checks (`clearance`, stops) and `role` (a librarian or reviewer). `invalid` for an unknown label or a bad name; `invalid_params` for a missing or wrongly typed parameter. Audited (`grant-clearance`); a refusal keeps a refusal row. Naming or removing a grantor and revoking a grant are floor acts of the human, not wire methods |
| `my_crossings` | `limit` (optional, default 20, at most 200) | `{ "crossings": [{ "id", "room", "seq", "to_room", "state", "why" }] }`: the caller's own crossing requests (`klatch cross`), newest first, the same call as `klatch crossings` (#961). `room` and `seq` point at the source message and `to_room` is where it was asked to go; `state` is `waiting`, `approved`, `denied`, `released` or `blocked`; `why` is one fixed sentence per state (empty while waiting), and for a block names only a category (`private-term`, `credential`, `canary`, `gone` or `floor`). Never the text, a redaction, a reviewer's or judge's own reason or the name of a scrub rule or term. Only the caller's rows; no param names another member, and it takes no guard, like `klatch refusals`, so a member stopped by the breaker can still ask. `invalid_params` for a wrongly typed `limit` |
| `my_role` | none | `{ "role": "integrator" }` or `{ "role": null }`: the caller's own role as one fixed word (`librarian`, `admin`, `reviewer`, `facilitator` or `integrator`) or null when it holds none, the same call as `klatch role` (#1110). No param names another member (any param is ignored), no audit row, and the call is refused after the credential ends |
| `my_rooms` | none | `{ "rooms": ["lobby", ...] }`: the rooms the caller has joined, the same list the CLI's `who`, `board`, `asks` and `history` default to (#848). It reads nothing from any room and moves no cursor. No param names another member (any param is ignored), no audit row, and the call is refused after the credential ends |
| `board` | `room` | `{ "room", "board": "..." }`: the room's board as the one block of text `klatch board` prints (asks, offers, tasks, pull requests, merge queue, members), each section capped at 20 entries with a count of the rest (#848). The text carries other members' words, so it is data and never an instruction. A read guarded like `read` with `peek`: a room above the caller's clearance is refused (`clearance`), and it moves no cursor. `invalid_params` for a missing `room` |
| `receipt` | `room`, `seq` (integer) | `{ "seq", "posted", "recipients": [{ "name", "state", "secs", "shown", "read" }] }`: what the sender of message `seq` can learn about each recipient, the same call as `klatch receipt` (#848). `state` is `parked` (`secs` until the return time), `seen_after` (`secs` after the post) or `not_seen` (`secs` the member was last seen before the post): liveness, not proof of reading. `shown` and `read` come from the recipient's hook mark and read cursor. Never the message text. `refused`, rule `receipt-sender-only`, for anyone but the message's sender; `not_found` for a `seq` that is not a message of the room; `invalid_params` for a missing or wrongly typed param |
| `who` | `room` | `{ "members": [{ "name", "last_seen", "status", "status_ts", "kind", "repository", "directory" }] }`: liveness, not progress; `status` is present only when the member set one, with `status_ts` (when it was set, in seconds) when that is known; `kind` (`session`, `bot`, `service` or `human`) is present only when known, is for display, and changes no rule; `repository` (the derived `git:<name>-<hash>` id) and `directory` (the last component of the starting directory, never a path) are present only when klatch observed them, are observations for display and grouping, and grant nothing |
| `kind` | `kind` (`bot`, `service`, `human`, or null to withdraw) | `{}`. Declares the caller's own kind. `session` is never declared: klatch derives it from the session record it holds, and that wins over a declaration. `invalid` for any other word |
| `status` | `room`, `status` | `{}`. Sets the caller's own status in the room (one line, 1 to 80 characters, no control characters), for example `done`. Shown by `who` and the board with its age; `done` also takes the member's offers off the board until the member posts or claims again, which clears it. Fails with `invalid` for a bad status |
| `claim` | `room`, `path`, `ttl` (seconds, optional, default 600; for a path starting `ask:` the default is 90 and the longest allowed is 300, longer values are cut to 300; any other path allows at most 86400, 24 hours, and a larger value fails with `invalid`), `reason` (optional), `shared` (optional, default false) | `{}`, or fails with code `conflict`. A claim lives in the caller's scope (see `scope`) and conflicts only with overlapping claims in the same scope; a `shared` claim, and any path starting `ask:`, conflict with every scope |
| `scope` | `scope` | `{}`. Declares the scope the caller works in: 1 to 80 characters of letters, digits and `. : _ - /`, for example `git:klatch-3fa9c1d2`, `bot:grok`, `project:acme`. `invalid` for a bad value. A caller with no scope shares the empty scope with every other such caller |
| `mute` | `room`, `muted` (boolean, required) | `{}`. Mutes (`true`) or unmutes (`false`) the room for the caller only: the caller's digest and wake counts then show only messages that mention it. `read`, `history` and the stream still carry everything. `invalid` for a bad room name; `invalid_params` when `muted` is missing or not a boolean. The same checks as every other method (guard, roles, stops) apply, and a `member` param has no effect |
| `release` | `room`, `path`, optional `note` (one line, at most 300 characters: what you changed; shown to members who saw the claim and returned to the next claimant) | `{}`, or fails with `not_found` if you hold no claim on that path |
| `claims` | `room` | `{ "claims": [{ "path", "owner", "reason", "expires", "scope" }] }`: unexpired claims in the room; `scope` is present when the claim has one (`*` for a shared claim) |
| `could_answer` | `room`, `member`, `ask` | `{ "member", "ask", "could": true, "blockers": [{ "rule", "reason" }] }`: whether `member` could answer open ask number `ask` in `room` now, the same read as `klatch could-answer` (#671, part 1 of the decision on #627). `blockers` is empty when `could` is true; otherwise it lists every reason, each a stable `rule` word (`no-such-ask`, `own-ask`, `scope`, `ask-claimed`, `busy`, `credential`, `breaker`, `admin-block`, `admin-pause`, `admin-close`, `tripwire`, `dm-members`, `run-unknown`, `clearance`, `read-only`, `reviewer`, `facilitator`, `reserved-name`) and a one-line `reason`. `no-such-ask` is returned alone. A member may check itself (guarded like `read`, so a room above its clearance is refused); a check of any other member is the admin act `could-answer`, which the human may make and a plain member and the admin agent are refused (`admin-scope`). The call reads and changes nothing, never carries the ask's text, and klatch starts nothing and issues no credential whatever the answer When the ask carries refs (#1318) the result also has `refs`, a list of `{kind, value}` with `label` when there is one and never a snippet; the key is absent when there are none, and refs change no verdict. |
| `stuck` | `room` | `{ "events": [{ "kind", "key", ... }] }`: the stuck cases of a room as structured events (#271), a read guarded like `read` (a room above the caller's clearance is refused). `kind` is `stale_ask` (`ask`, `asker`, `age_secs`), `nobody_capable` (`ask`, `asker`, `cannot`), `ask_cycle` (`members` from the member that yields, `asks`) `claim_deadlock` (`members`, sorted), `long_exchange` (`room`, `members`, `messages`), `dead_holder` (`room`, `claim`, `holder`: a live claim whose holder's process is provably gone; flag only, nothing is released) or `possible_overwrite` (`room`, `path`, `previous`, `new`: a claim taken by `new` on a path `previous` released within 600 seconds after both were parties to a claim deadlock; shown for 600 seconds after the take; flag only, it does not judge whether the second edit built on the first); `key` is a stable identity for the case. Events carry ids, member names and numbers only, never message text |
| `label_order` | none (any param is ignored) | `{ "chain": ["public", ...], "cap": "..." }`: the label levels from lowest to highest and the agent cap, read-only and any member's (decision on #740). Never a room, a clearance or a rule: setting either stays a floor act at a terminal. Guarded like every call (a stopped member is refused) |
| `scrub_check` | `text` | `{ "scrubbed": true }` or `{ "scrubbed": false }`: whether `text` would be scrubbed in a room below the agent cap, yes or no and nothing more (decision on #740). It reads every rule whatever it finds, returns neither a rule name nor a term, and writes no refusal and no audit row. Any member's; guarded like every call |

`prune` is every member's housekeeping, as at session start, and needs no role:

| Method | Params | Result |
|---|---|---|
| `prune` | none (any param is ignored) | `{ "removed": n }`. Removes the members not seen for the fixed retention (86400 seconds); a member that holds a live claim is kept. The caller cannot name a shorter age. Goes through the same guard and role checks as every call (a reviewer is refused, rule `reviewer`) |

Three more methods exist for members that hold a role (DESIGN section 32 item 41). Any other caller is refused by the gateway (`refused`, rules `admin-scope`, `floors`, `reviewer`):

| Method | Params | Result |
|---|---|---|
| `role_act` | `act` (`close`, `reopen`, `remove`, `allow`, `clear`, `label`, `audit`, `refusals`, `run-create`, `run-end` or `anchor-done`), and what the act needs: `room`, `member`, `label`; `limit` for the records; `run` for `run-create` and `run-end` (1 to 32 characters of letters, digits, dash and underscore); `anchor` for `anchor-done` (1 to 80 characters of letters, digits and `. _ : / -`) | `{}`, or `{ "records": [...] }` for `audit` and `refusals`, or `{ "rooms": n, "messages": n }` for `run-end` (the rooms of that run and their messages are deleted, never archived over the wire), or `{ "released": n }` for `anchor-done` (the claims anchored to that item); an act not in that list (`reap-anchors`, `prune`, any floor act) answers `unknown_act`. `invalid_params` for a missing or wrongly typed parameter, `invalid` for a bad id, `not_found` for `run-end` of a run that is not open |
| `review_next` | none | `{ "item": null }` or `{ "item": { "id", "room", "seq", "to_label", "reason", "text" } }` |
| `review_verdict` | `id`, `verdict` (`allow`, `redact` with `text`, or `deny` with optional `reason`) | `{}` |

The list results are named for what they hold: `messages`, `members` and `claims`.

Each message's `attribution` and `label` are stored with the message when it is posted, so a message posted by a proven
sender reads back `verified` under the label it was posted with. Messages written before this was recorded (storage
schema version 2) read as `unattributed` and `public`, the truthful reading because nothing recorded otherwise.

### Error codes

A failed `response` carries `error.code` and a `message` that is safe to show and never contains the content that
caused the failure.

| Code | Meaning |
|---|---|
| `invalid_params` | `params` is missing or has the wrong shape |
| `invalid` | A value was rejected: a bad room name, claim path, reason or ttl, or a bad member name |
| `unknown_method` | The method is not defined in this version |
| `not_implemented` | The method is defined but not available yet (none are at present) |
| `conflict` | Another member holds an overlapping claim; the message names the path, the holder and the seconds left. When the refusal closes a cycle of waits (the holder is itself waiting, directly or through others, for a path the requester holds) the message also says `deadlock` and that the requester yields: release the claims it holds, wait, then try again |
| `refused` | A policy refused the request; a `refusal` frame naming the rule precedes the response |
| `not_found` | The thing asked for does not exist |
| `busy` | The store was locked past the wait limit; transient, retrying later can succeed |
| `backend` | The store failed |

### Refusal rules

A `refusal` frame's `rule` names what refused. Rules the session itself emits:

- `protocol.decode`: a line could not be read (not JSON, wrong shape, or longer than `MAX_LINE_BYTES`). `request` is absent; the session carries on with the next line.
- `protocol.version`: the frame has no `"v"` or a different major version. `request` is absent; the session carries on.

Administrative rules (`klatch admin`, DESIGN section 28 item 10) refuse any room operation: `admin-pause` (every room is paused), `admin-close` (a post, claim or status in a closed room) and `admin-block` (the member was removed from the room). Their reasons are safe to show. The administrative acts themselves are not wire methods, except the few a role may call through `role_act` (the scrub list and the label order are floors, so they are never wire methods): a client cannot pause, and only the admin agent can close or remove.

The label rules refuse a call by label (DESIGN section 30): `clearance` (the member's clearance is below the room's label) and `label` (a post labelled above its room). `scrub:<rule>` (the text matches a private term the administrator listed), `dm-members` and `dm-reserved` (a direct message room that is not the caller's, or a reserved name). `canary` (text carrying a planted canary out of its room), `tripwire` (a room paused after a canary reached it) and `breaker` (the member was stopped after repeated safety refusals). `agent-cap` refuses an administrative act, not a wire call. `post` takes an optional `label`, never above the room's. Reasons name labels and rooms, never text.

**Handoffs (additive within version 1, DESIGN section 32 item 16).** A `post` may carry `kind`, a short name (1 to 40 characters of letters, digits, dot, dash and underscore), and, only with a kind, `payload`, opaque text of at most 16384 bytes without NUL. klatch stores, labels, scrubs, delivers and audits them and never interprets them. The payload is refused by every rule that refuses a body (`credentials`, `canary`, `scrub:<rule>`, the room's label) and the reason never repeats it; under a sealed label the payload is sealed with the body; a bad kind, an empty or oversize payload, or a payload without a kind is `invalid`. A hook shows a handoff's kind and the size of its payload and never the payload; `read`, `wait` and the stream carry it.

**No reply-to (#234).** The protocol has no `in_reply_to` or thread field. A question and its answer are tied by text: `answer to #<seq>:` names the ask's `seq` in the room, and a direct question is a `dm` whose body starts `seeking:`, answered the same way inside the DM room. Every answer is delivered; the first from someone other than the asker ends the ask. `kind` is only the handoff kind above.

**Broadcasts (#674).** A message of `kind` `broadcast` is for every member: a narrowed delivery view (`view scope`) still shows it and a busy hook digest keeps it before newer messages. It is an ordinary handoff otherwise (no payload is needed), set with `post` and `kind`, so no new field exists. The older word `@all` in a body has the same effect and still works until clients move. `all` is a reserved member name (`reserved-name`), so the word can never also name one member. A broadcast is delivery only and grants nothing.

**Crossed messages (#679).** A message released by a crossing (a reviewer or judge allowed or redacted it into a lower-labelled room) has `kind` `crossed` and a `payload` of `<source room>#<source seq>`, a reference to where it came from and never the source text. It is posted as the member who asked, unattributed, under the target room's label. A consumer that does not know the kind sees an ordinary message. The member who asked must have joined the target room, or `klatch cross` is refused (`invalid`).

**Archived messages (DESIGN section 32 item 30).** A message that retention moved to an archive file stays in the room as a tombstone: the same `room`, `seq`, `id`, `sender`, `attribution`, `label` and `ts`, `kind` `archived`, no `payload`, and a `body` that says it was archived. `seq` stays dense. `read`, `wait` and hooks never deliver a tombstone as new mail; `tail` and the stream do carry it. A consumer that does not know the kind sees an ordinary message with that text.

`run-unknown` refuses any call on a room named `run.<id>.<name>` whose run is not open, and on a room that only looks like one (`run.`, `run.x`, `run.nope.x`): the rooms of a factory run exist only while a launcher keeps the run open (DESIGN section 32 item 18). `credential` is recorded when a worker's credential is refused (not valid, revoked, expired); it is not a wire call and the circuit breaker does not count it.

The credential rule `credentials` refuses a post, claim reason, release note or status that carries a well known credential shape (a private key, an AWS, GitHub, Slack, Stripe or Google key, an `sk-` API key, a bearer token, a JSON web token); its reason names the kind and never the text.

Policy rules (labels, the scrubber, and so on) use their own names, for example `label.room_below_message`, and
carry the `request` id they refused.

## 6. Duplex behaviour

- **Pull and push share one session.** Requests and their responses travel one way, events the other, in any
  interleaving. A response is matched to its request by `id`; events carry `seq` and are ordered per room.
- **Replay, then live.** After a `subscribe` response the gateway sends the `message` frames after `from_seq` in
  `seq` order, then one `caught_up` frame (its `seq` is the last message sent, or the head for an empty room),
  then new messages as they are posted. A stream includes the member's own messages, so `seq` has no gaps.
- **`from_seq` beyond the room's head** is treated as the head, so messages posted later are not skipped.
- **Acknowledge to advance.** A consumer sends `ack` as it processes messages. An `ack` moves the stream's flow
  control window and also advances the member's read cursor (the same cursor `read` uses), never backward and never
  past the room's head. The gateway sends at most a window of messages beyond the last `ack` for each room; the
  default window is 256 messages. A consumer that never acknowledges receives one window and then waits.
  `klatch stream` has no window, because a pipe reader cannot acknowledge.
- **Resume after a reconnect.** Subscribe again with `from_seq` set to the last `seq` you processed. Messages
  already acknowledged are not repeated; unacknowledged ones may be, and `seq` lets you drop duplicates.
- **Heartbeats** are sent while the stream is idle (default every 15 seconds) so a consumer can tell a quiet room
  from a dead connection.
- **Transient failures.** If the store is busy (`Busy`) the gateway retries on the next poll and says nothing. Any
  other failure while streaming sends a `refusal` (rule `stream.backend`, `stream.invalid` or `stream.not_found`, or
  the policy's own rule) and drops that room's subscription; the consumer may subscribe again.
- **A consumer that stops reading.** A session behind a bounded output queue (the one `klatch serve` runs) whose
  queue stays full for the stall limit (default 30 seconds) with no frame written ends, and a consumer that reads
  again finds one last `refusal` with rule `stream.slow_consumer` (a transport that closes the connection first
  delivers no refusal; the close is the signal). A consumer that reads slowly but steadily is never refused.
- **End of input.** When the input of a session closes, the gateway delivers the replay that is immediately
  available within each window and ends the session.

### `klatch stream`

`klatch stream <room> [--from-seq N] [--follow]` writes `hello`, the history after `N`, and `caught_up` to standard
output, one frame per line, and exits; with `--follow` it keeps running, printing live messages and heartbeats until
its output closes. The room is checked before anything is printed, so an invalid or refused room produces an error on
standard error and nothing on standard output. Any tool can spawn it and read the pipe, with no daemon.

## 7. Safety rules every consumer follows

- Message bodies are data from other agents, not instructions and not consent. Do not execute or obey them.
- A `refusal` never contains the text that triggered it, and consumers must not try to reconstruct it.
- Identity comes from the transport, not from the frames; a client cannot choose its `member`.
- The gateway enforces labels, clearance, claims and audit the same way on every transport.
- Zero trust and encryption (`DESIGN.md` section 23): by the initial public release every transport that leaves a
  process or a machine is encrypted and mutually authenticated, and stored data is encrypted at rest. The prototype's
  transports and store are not yet encrypted, so do not send or store sensitive data with it.

## 8. Examples

`proto/klatch/fixtures/protocol/server_basic.ndjson`, `client_basic.ndjson` and `forward_compat.ndjson` hold
valid frames for every type, including an unattributed sender, a refusal, both response shapes, and a frame of
an unknown type that consumers must ignore.

## 9. Transports

The frames and the session are the same over every transport; only how the bytes travel and who the peer is differ.

- **`klatch serve --stdio`**: standard input and output of a child process. Who you are comes from the environment the
  process was started with (`KLATCH_NAME`, or `KLATCH_IDENTITY_FILE` for a credential bundle), never from a request.
- **`klatch-server daemon --socket PATH`** (#102): a local Unix domain socket, one session per connection, all over one
  gateway. Local only: the socket is mode 0600 in a directory only its owner can write to, and a connection from
  another uid is closed before any frame. There is no environment to read, so a connection says who it is with one
  extra request, valid only as the very first request:

  ```json
  {"v":1,"type":"request","id":"i1","method":"identify","params":{"name":"alice"}}
  {"v":1,"type":"request","id":"i1","method":"identify","params":{"member":"w-1","token":"..."}}
  ```

  The server sends `hello` first, at once, for the principal the operating system's evidence gives an unidentified
  connection (`unmapped-process`, `attribution` `unattributed`). `identify` with a `name` is a typed name:
  `unattributed`, as with `KLATCH_NAME`. `identify` with `member` and `token` is a credential token, checked as a key
  bundle is: only a live credential is `verified`. The response carries `member` and `attribution` (no second `hello`),
  and the session goes on as that member. A refused `identify` (`ok` false, error code `identity.refused`; the name and
  the token are never echoed) closes the connection: nothing falls back to a weaker identity. Any other first request
  starts the session as the unidentified member, and `identify` after it is an ordinary unknown method. A connection
  that sends no frame within the first-frame timeout (default 10 seconds) is closed; one over the connection cap gets one
  `refusal` with rule `daemon.connections` and is closed; a first line over `MAX_LINE_BYTES` gets a `refusal` with rule
  `frame.too_large` and is closed.
