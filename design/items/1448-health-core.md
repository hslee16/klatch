Code health K07: klatch core, client, session, jev and commands (#1448; no schema change; 2026-11-23).
Behaviour-preserving cleanup of the core modules named by the CodeScene inventory (epic #1441). No SQL, schema, stored byte, wire format, CLI output, hook output byte or `pub` signature changed; the public paths of the `client` types are kept by `pub use` in `client.rs`. No test was added: this is a pure refactor, so there is no red test, and the existing tests that pin each path (hook, wait and read output, the session wire, the jev client, request and transport) are the characterisation. The test count of every split test file is the same before and after (jev request 24, jev tests 47). `commands/agents.rs` went from 560 to 301 lines; no file under `commands/` is over the 600 line cap (`messaging.rs` is 578).

Functions changed:
- `jev::request`: `Render.entry` (`leaf`, `sentence`, `list`, `object`), `Render.question` (`choice`, `score`, `noul`), `build_request` (`check_question_ids`, `asset_tables`, `sent_len`).
- `commands::agents`: `session_start` (`start_of`, `briefed_rooms`, `briefing_text`, `own_scope_differs`), `pre_tool_use` (`refuse_unnamed_subagent`, `held_claims`, `judged_words`), `hook` (`note_activity`, `stop_hook`, `hook_text`, `print_hook`, `append_line`).
- `client`: `Client.start` takes a `Link` (writer, child, shutdown) instead of three arguments.
- `session`: `Session.run` (`refuse_lost`), `Session.stream` (`stream_step`), `methods::handle` (`handle_talk`, `handle_rooms`, `handle_claims`, `post_by_shape`, `read_json`), `members_json` (`member_json`, `place_json`).
- `jev`: `Image.from_bytes` (`sniff`, `check_dimensions`), `NoulThresholds.new` and `pick`, `date_ok`, `Target.parse` (`check_authority`), `HttpTransport.send` (`call`, `is_encoded`), `run_with_retry` (`wait_within_budget`).
- `commands`: `capped::read` (`mention_note`, `next_read_command`), `print_woken` (`gather_woken`, `wait_omitted_note`, `mark_woken`), `wait::dm_help_lines` (`dm_reply_line`, `dm_nudge_line`), `asks_note_lines` and `room_briefing` (argument tuples), `wake` (`wake_pointer`), `run_work` (`crossings`, `could_answer`), `run_messaging` (`ask_command`, `post_command`, `grant`), `ask_refs::build_refs` (`check_counts`, `one_ref`), `mcp` wait (`woken_json`).
- The live jev check `jev_live_all` runs in stages (`live_models` to `live_burst`); it is `#[ignore]`d (needs a key and the real API), so it was compiled but not run.

Files moved, no code changed: `client.rs` into `client/records.rs`, `client/requests.rs`, `client/methods.rs`; the renderer of `jev/request.rs` into `jev/request/render.rs`; `session_start` and `pre_tool_use` of `commands/agents.rs` into `commands/hook_start.rs` and `commands/hook_tool.rs`; the tests of `jev/request`, `jev`, `jev/decision` and `jev/retry` into `tests/` directories by topic; `session/tests/wire.rs` into `wire.rs` and `wire_guard.rs`.

Left: `build_request` (5 arguments), `RetryPolicy::new` (6), `pick_screened` (5) and `advise_route` (8) are public signatures, so their argument counts stay. The integration tests under `proto/klatch/tests` belong to #1453; the only edit there is one path in `tests/client_methods.rs` (the Rust wrapper check now reads `src/client/methods.rs`).

| File | Before | After |
| --- | --- | --- |
| jev/request.rs | 7.16 | 9.68 (build_request has 5 arguments, public) |
| jev/request/tests.rs | 7.42 | 10.0 (split into tests/*.rs) |
| session/tests/wire.rs | 8.03 | 10.0 |
| client.rs | 8.28 | 10.0 |
| commands/agents.rs | 8.37 (7.76 at the start of this work) | 10.0 |
| jev/tests.rs | 8.54 | 10.0 (split into tests/*.rs) |
| jev/http/live.rs | 8.74 | 10.0 |
| session/methods.rs | 8.77 | 10.0 |
| jev/image.rs | 8.81 | 10.0 |
| session/mod.rs | 8.87 | 10.0 |
| commands/work.rs | 8.96 | 10.0 |
| commands/capped.rs | 9.0 | 10.0 |
| jev/decision.rs | 9.09 | 9.68 (pick_screened has 5 arguments, public) |
| commands/wait.rs | 9.24 | 10.0 |
| jev/http/tests.rs | 9.24 | 9.24 (not split: tests/no_telemetry.rs allows loopback servers in that one file only, and widening it was not worth the score) |
| commands/ask_refs.rs | 9.38 | 10.0 |
| jev/decision/tests.rs | 9.38 | 10.0 (split) |
| jev/models.rs | 9.38 | 10.0 |
| jev/retry/tests.rs | 9.38 | 10.0 (split) |
| commands/messaging.rs | 9.46 | 10.0 |
| jev/http.rs | 9.48 | 10.0 |
| jev/retry.rs | 9.53 | 9.68 (RetryPolicy::new has 6 arguments, public) |
| commands/briefing.rs | 9.68 | 10.0 |
| jev/facilitate.rs | 9.68 | 9.68 (advise_route has 8 arguments, public) |
| session/mcp.rs | 9.68 | 10.0 |
| new: client/{records,requests,methods}.rs, jev/request/render.rs, commands/hook_{start,tool}.rs, session/tests/wire_guard.rs, the new tests/ files | n/a | 10.0 each |
