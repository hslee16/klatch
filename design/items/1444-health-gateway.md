Code health K03: gateway production modules (#1444, part of epic #1441; no schema change; 2026-11-23).
Behaviour-preserving cleanup of the gateway modules named by the CodeScene inventory. No SQL, schema, stored byte, wire format, CLI output, error text, audit row or `pub` signature changed; the golden tests of #1480 are unchanged (never regenerated) and pass before and after every step. The guard order of every check is kept: authorization, then validation, then effect, and each early return is where it was. Where a split would have merged or reordered a short-circuit that reads the store (`?` inside an `||` chain), it was written as sequential early returns in the same order.

Tests: one characterisation test file was added first, `gateway/gateway/tests/role_order.rs` (6 tests), for `check_role` (Complex Method, security critical). It pins these orders by tripping two checks at once: reserved name before reviewer; reviewer before the merge claim; the facilitator bound before the merge claim and before the need-info state rule; the merge claim before the librarian's read-only limit; the member admin rule before the admin agent's list; the room capability before the role limit (added after review, which swapped the two and found nothing red; the new test fails on that swap, checked in a scratch copy and reverted). It was green on the old code; a pure refactor has no red test. The rest of the changed functions are pinned by the existing gateway tests and the goldens. Test count: 3410 before, 3416 after (the six new tests); the goldens and `tests/authz_audit.rs` pass.

Functions changed:
- `priority`: `Gateway.importances` (cc 20, bumps 3, 74 lines) split into `answerable_asks`, `own_asks_pointed_at` (`is_ask_of`), `coordinators_among` and a private `Reach` with `class_of`, `is_ask_for_me`, `answers_own_ask`. The store reads happen in the original order.
- `gateway.rs`: `post_labeled` and `post_handoff` share `post_and_route` (the duplicated post-then-route body); `post_ask` takes its scope and refs checks from `check_ask_shape` (same order: member, scope, ask form, then the room checks).
- `roles`: `check_role` is a list of steps (`check_reserved`, `check_facilitator_bound`, `check_role_limit`) in the original order; `merge_claim_refusal` takes the integrator or human test from `may_take_merge_claim`.
- `dm`, `dm_exchange` (new file, code moved): `long_exchange` and `exchange_event` moved to `dm_exchange.rs` (file complexity of `dm.rs` was the limit); the `exchange_event` conditional is two sequential checks (`both_in` helper) with the store read last.
- `facilitator`: `check_need_info`, `check_facilitator_dm`, `audit_facilitator_dm` take a `NeedInfo` (to, room, body) instead of three strings.
- `grantor`: `check_grant_rules` takes a `Grant` (room, member, label).
- `board/digest`: `digest_with` split into `digest_room` and `room_backlog` (a `Walk` carries who, backlog, now); the refusal and `continue` stay in the loop, before any read of the room.
- `machine_checkpoint`: `write_machine_checkpoint` split into `room_machine_state`, `status_in`, `is_quiet` (an `Own` carries member, trail, end word, now).
- `board/could_answer`: `claim_blockers`, `third_person` and the crate-internal `could_answer_of` take tuples (callers in `ask_routing.rs` changed by two lines).
- `history`: `history` split into `newest_visible`, `visible_after`, `is_hidden`.
- `workers`: `check_spec` takes the two lifetime checks from `check_lifetimes` (order: name, ttl, cap, parent).
- `tripwire`: `plant_on_label` takes its three-part condition from `plants_on_label` as sequential checks.
- `board/tasks`: `attach_machine_checkpoints` split (`attach_checkpoint`, `stand_in_task`).
- `shared`: `SharedGateway.start` split into `build_gateway` and `serve` (the owner thread body; same panic handling, same key load and unload around each request).

Not pinned by any test (found by an independent review with mutations; no code added here):
- The key reset after a keyed request: `serve` in `shared.rs` calls `use_label_keys(LabelKeys::new())` after a keyed job.
- The `ask.sender == me` test in `priority.rs` `is_ask_of`: a message that is not the caller's own ask does not count as one.
- Two pure read orders (a swap changes no result): in `plants_on_label`, `is_admin_agent` is read before `canary_keys_of`; in `exchange_event`, `dm_pair` is read after the length and membership checks.

Left alone, and why:
- Public signatures with 5 arguments (`mark_shown`, `post_ask`, `dm`, `grant_room_clearance`, `admin_set_room_grantor`, `admin_set_room_clearance`, `invite`, `request_crossing`, `admin_set_ssh_key`, `could_answer`) and the `Messages`, `Access`, `Board` and `Claims` trait methods of `transport/*.rs`: the arguments are the public API (used by `tests/`, the server and the bench), so the counts stay.
- "String Heavy Function Arguments" in `grantor.rs`, `rooms.rs`, `github_anchor.rs`, `room_require.rs`, `transport/access.rs`, `transport/board.rs`, `transport/claims.rs`: a module-wide share of `String` arguments (48 to 50 percent) on public methods; fixing it needs newtypes in the public API.
- `transport/messages.rs` Code Duplication (`SharedTransport.dm`, `post_labeled`, `post_scoped`): the shared `Own::own` and `Lend::lend` shape is how a request crosses to the owner thread; a macro would hide the guard calls, so it stays.

| File | Before | After |
| --- | --- | --- |
| gateway/priority.rs | 8.07 | 9.68 (mark_shown has 5 arguments, public) |
| gateway.rs | 8.54 | 9.68 (post_ask has 5 arguments, public) |
| transport/messages.rs | 9.09 | 9.09 (left, see above) |
| gateway/dm.rs | 9.24 | 9.68 (dm has 5 arguments, public) |
| gateway/roles.rs | 9.36 | 10.0 |
| gateway/facilitator.rs | 9.38 | 10.0 |
| gateway/grantor.rs | 9.38 | 9.38 (the three public methods) |
| gateway/rooms.rs | 9.38 | 9.38 (invite, public) |
| gateway/board/digest.rs | 9.48 | 10.0 |
| gateway/machine_checkpoint.rs | 9.6 | 10.0 |
| gateway/board/could_answer.rs | 9.68 | 9.68 (could_answer has 5 arguments, public; one issue fewer) |
| gateway/history.rs | 9.68 | 10.0 |
| gateway/tripwire.rs | 9.68 | 10.0 |
| gateway/workers.rs | 9.68 | 10.0 |
| gateway/board/tasks.rs | 9.84 | 10.0 |
| shared.rs | 9.84 | 10.0 |
| crossing.rs, github_anchor.rs, room_require.rs, ssh_key.rs, transport/access.rs, board.rs, claims.rs | 9.68 | 9.68 (public signatures, see above) |
| new: gateway/dm_exchange.rs, gateway/tests/role_order.rs | n/a | 10.0 each |
