Code health K01: sqlite stats and usage_stats (#1442, part of epic #1441; no schema change; 2026-11-23).
Behaviour-preserving cleanup of the usage report code and its tests. No SQL text, schema, stored byte, wire format, report output or public signature changed. The collision named on the ticket (#1401, #1403, #1407 touching these files) was checked: those tickets were open but no branch or pull request was in flight on `stats.rs` or `usage_stats.rs`, and the three counters they added were already in main, so the work started.

Tests: a pure refactor, so no test was red first. A golden test (`store/sqlite/tests/stats_golden.rs`, files in `tests/golden/`) renders the text and JSON of the claims bench under four option sets (all, one room, since a day with top 2, one room with since and top 1) and compares them with files recorded from the code before the change. It was verified against the original code first (passes), and passes after. The existing `tests/stats.rs` assertions are kept as the net; the same facts are asserted after regrouping.

Functions changed. `store/sqlite/stats.rs` was one 721 line file with `collect` at 184 lines. It is now the shared helpers (`present`, `has`, `has_column`, `rows`, `one`, `name_counts`, `printable_counts`, `words`, `class_counts`) and `collect`, which calls `sealed_labels`, `fill_days`, `fill_senders`, `claim_counts`, `fill_hooks` and `fill_credentials`. Four child modules hold the sections: `stats/rooms.rs` (`room_labels`, `lowest_label`, `class_of`, `groups`, `fold_groups`), `stats/asks.rs` (`ask_words`, `created_asks`, `answer_delay`, `closed_ask`, `answers_and_closes`, `with_scope`, `with_refs`; the tuples became the `AskWord` struct), `stats/members.rs` (`posters`, `names_where`, `with_parent`, `never_read_by_day`) and `stats/events.rs` (`claim_history`, the machine, passive and routing counters, with a shared `counter` and `WINDOW`). `domain/usage_stats.rs` keeps the types and vocabulary helpers; `to_text` and `to_json` moved to `domain/usage_stats/render.rs` as one function per section. `domain/mod.rs` is untouched. Tests: `tests/stats.rs` was split into `stats.rs`, `stats_events.rs` and `stats_fixture.rs` (`Msg` builder replaces the ten argument `msg`; `bench` is data driven; `assert_counts` groups runs of asserts).

| File | Before | After |
| --- | --- | --- |
| store/sqlite/stats.rs | 6.62 | 10.0 |
| store/sqlite/stats/asks.rs, events.rs, members.rs, rooms.rs (new) | n/a | 10.0 each |
| store/sqlite/tests/stats.rs | 6.83 | 10.0 |
| store/sqlite/tests/stats_events.rs, stats_fixture.rs, stats_golden.rs (new) | n/a | 10.0 each |
| domain/usage_stats.rs | 8.39 | 10.0 |
| domain/usage_stats/render.rs (new) | n/a | 10.0 |

The scores on the ticket (6.63, 6.83, 8.86) were from the inventory; `cs review` on the same commit gave 6.62, 6.83 and 8.39. The 600 line cap does not apply to these directories and every new file is under it anyway.
