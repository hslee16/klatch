Code health K05: domain, dm, mediator, credentials and the remaining modules (#1446; part of #1441; 2026-11-23).
Behaviour-preserving cleanup of the domain modules named by the CodeScene inventory. No schema, SQL, stored byte, wire or public API change; every `pub` signature is unchanged (so `stuck_events` keeps its five arguments). A pure refactor has no red test: the existing unit tests pass before and after, and four characterisation tests were added to `credentials.rs` first (they passed on the old code and pass on the new): the spaced-run width, word count and length boundaries, the byte range of a run and two runs in one text, the length floors of command passwords (`curl -u`, `mysql -p`, `--password=`) and the URL password read from the authority only.

Functions changed: `dm.rs` `crossing_exchange` (`exchange_peer`), `clarification_round` (`second_round_start`, `is_second_round`, `is_clarification_of`); `subagent_name.rs` `scan` (`scan_simple`, `scan_dash_c`, `is_prefix_word`) and `segments` (a `Splitter` state struct); `mediator.rs` `propose` (`post`, `ask_cycle`, `claim_deadlock`); `credentials.rs` `spaced_runs` (`run_end`, `next_in_run`) and `url_or_command_password` (`url_password`, `curl_user_password`, `mysql_flag_password`, `password_flag`, `real_password`); `scope.rs` `claim_scope` (`is_room_wide`); `stuck.rs` `stuck_events` (`unanswered_ask_events`); `router.rs` `plan` (`route_for`, `invites_for`, `works_on`, `is_live`); `cascade_judge.rs` `CascadePolicy::new` (`is_unit_interval`); `receipt.rs` `receipt_of` (`joined_after`); `wait_filter.rs` `resolve_dm_targets` (`dm_room_target`); `wake_cap.rs` `fit_prefix`. Large assertion test blocks in dm, subagent_name and mediator were split or made table driven (`check`, `assert_text`, `crossing`, `round`).

| File | Before | After |
| --- | --- | --- |
| domain/dm.rs | 7.66 | 9.68 (String Heavy Function Arguments left) |
| domain/subagent_name.rs | 7.83 | 10.0 |
| domain/mediator.rs | 8.8 | 9.68 (String Heavy Function Arguments left) |
| domain/credentials.rs | 8.81 | 9.68 (String Heavy Function Arguments left) |
| domain/scope.rs | 9.24 | 9.68 (String Heavy Function Arguments left) |
| domain/stuck.rs | 9.38 | 9.68 (public `stuck_events` arity left) |
| domain/router.rs, cascade_judge.rs, receipt.rs, wait_filter.rs, wake_cap.rs | 9.63 / 9.68 | 10.0 |

Left alone: `convention_words.rs` and `text.rs` (9.68, String Heavy Function Arguments only, which is the nature of text helpers), `domain/mod.rs` (K06 owns it).
