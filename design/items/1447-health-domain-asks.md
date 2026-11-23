Code health K06: domain asks, hook, briefing, tasks, merge queue and checkpoint, long functions and large test files (#1447; no schema change; 2026-11-23).
Behaviour-preserving cleanup of the domain modules named by the CodeScene inventory (epic #1441). No SQL, schema, stored byte, wire format, hook output byte or `pub` signature changed; `domain::outside_view_line` and the other re-exports of `hook` keep their paths. Two characterisation tests were added (`relative_paths_and_secret_params_keep_their_exact_rules` for `ask_refs`, `civil_days_counts_real_calendar_days_only` for `asks`); the second was run against the unrefactored code and passes there too, so a pure refactor has no red test. The existing hook, briefing, merge queue and task tests are table-like and unchanged in what they assert; the hook test count (63) and the asks test count (41 plus the new one) are the same before and after the file moves.

Functions changed: `ask_refs::relative_path` (`is_line_range`, `without_line_range`, `plain_segments`), `url_credential` (`credential_in_url`), `secret_param` (`SECRET_WORDS`, `key_names_a_secret`); `machine_checkpoint::machine_body` (`fixed_parts`, `files_part`, `joined`), `repo_relative` (`keepable`); `briefing::render_briefing` (`present_sections`, `fit_sections`); `merge_queue::merge_queue` (`apply_ready`, `apply_state`, `next_state`, `active_move`, `arranged`, a `StateWord` value); `tasks::Task::line` (`flatten`, `clipped`, `standing_notes`), `open_tasks` (`start`, `checkpoint`, `hand_over`, `finish`, `apply`); `asks::civil_days` (`days_in_month`, `ymd`, `is_real_day`). Test files: the assertion blocks of the machine checkpoint and briefing tests became tables and `shows`/`hides` helpers; the long hook fence test builds its input in `hostile_digest`.

Files moved, no code changed: the tests of `asks.rs`, `ask_refs.rs` and `hook.rs` left their files (`asks/tests/`, `ask_refs/tests.rs`, `hook/tests/`, split by family into files under 600 lines); the section renderers of `hook.rs` (`render_claims` to `other_units`) moved to `hook/sections.rs`, so `hook.rs` is 505 lines and no file of this cluster is over the 600 line cap.

| File | Before | After |
| --- | --- | --- |
| domain/ask_refs.rs | 7.90 | 9.68 (String Heavy Function Arguments left: many small `&str` validators) |
| domain/machine_checkpoint.rs | 8.28 | 9.68 (same left) |
| domain/briefing/tests.rs | 8.81 | 9.68 (same left) |
| domain/briefing.rs | 9.09 | 10.0 |
| domain/merge_queue.rs | 9.09 | 10.0 |
| domain/tasks.rs | 9.22 | 9.68 (String Heavy Function Arguments left) |
| domain/asks.rs | 9.24 | 10.0 |
| domain/hook.rs | 9.54 | 10.0 |
| new: ask_refs/tests.rs, asks/tests/*.rs, hook/sections.rs, hook/tests/*.rs | n/a | 10.0 each |
