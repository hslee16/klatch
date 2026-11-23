Code health K13: gateway tests and test_support (#1454, part of epic #1441; test code only, no production change; 2026-11-23).
Behaviour-preserving cleanup of the gateway test files named by the CodeScene inventory. No production file, golden test or assertion was changed, weakened or removed: the same checks run, grouped into named helpers or loops. Nothing was red before: all 838 gateway tests of the `klatch` lib passed first (`cargo nextest run -p klatch --lib gateway`), and `cargo nextest list` names were compared before and after.

Tests: 838 gateway tests before, 839 after (2526 and 2527 in the whole lib). The one new test, `priority::an_answer_to_an_ask_that_is_not_mine_is_not_an_answer_to_my_ask`, fills a gap named in the #1444 note: it pins the `ask.sender == me` test in `is_ask_of`. It is green on the code and red when that condition is removed (checked on a mutated copy and reverted). Every other test name is unchanged (a sorted diff of the lists shows only that line).

Files changed:
- `gateway/tests/facilitator.rs`: the 98 line attempt table split into `room_attempts` and `admin_attempts`; four assertion blocks moved into `assert_it_may_look_at_the_lobby`, `assert_room_creation_audited`, `assert_nothing_changed`, and loops over the refused results.
- `claude_judge_tests.rs`: assertion blocks moved into `assert_quarantine_flags`, `assert_merge_text_on_stdin_only`, `assert_child_env_is_clean`, `assert_is_deny`, `assert_scrub_prompts`, `assert_gate_and_budget`; a label loop; `flag_value` replaces a closure.
- `shared_fairness_tests.rs`: the ignored measurement split into `print_alone`, `slow_requests`, `print_beside`; `fill` takes `(room, tag)` as one pair. The ignored measurement was run once and passes.
- `gateway/tests/lease_settings.rs`, `librarian.rs`, `tripwire.rs`: duplicated bodies shared through `sets_and_reads`, `takes_only`, `a_former_member_made_librarian`, `eight_refusals_leave_the_breaker_alone`. The librarian wait test now also asserts what the look test asserted (the former member has a last seen), a small strengthening.
- `gateway/tests/zero_trust.rs`: the nested match of the ended credential test moved into `assert_after_end`.
- `test_support.rs` (727 lines, over the 600 cap) became `test_support/mod.rs` (the spy store core, the spy guard), `messages.rs`, `claims.rs`, `admin.rs` (the trait impls, code moved as is). A stray doc comment of `inner_join_for_test` that sat on the wrong function moved to it.
- `gateway/tests/priority.rs`: the one new test.

Not pinned still (no test added): the key ring reset in `shared.rs` `serve` (needs a keyed request across two jobs on one owner thread), and the two pure read orders (a swap changes no result).

Left alone: the "Excess Number of Function Arguments" of `SpyStore` methods in `test_support/claims.rs` and `messages.rs` are the signatures of the store traits (5 or 6 arguments), so they stay.

| File | Before | After |
| --- | --- | --- |
| gateway/tests/facilitator.rs | 8.88 | 10.0 |
| claude_judge_tests.rs | 9.09 | 10.0 |
| shared_fairness_tests.rs | 9.25 | 10.0 |
| gateway/tests/lease_settings.rs | 9.38 | 10.0 |
| gateway/tests/librarian.rs | 9.38 | 10.0 |
| gateway/tests/tripwire.rs | 9.38 | 10.0 |
| gateway/tests/zero_trust.rs | 9.38 | 10.0 |
| test_support.rs (measured 8.54 today, not 9.68) | 8.54 | mod.rs 10.0, admin.rs 10.0, claims.rs 9.68, messages.rs 9.68 (trait signatures) |
| gateway/tests/priority.rs | 10.0 | 10.0 |
