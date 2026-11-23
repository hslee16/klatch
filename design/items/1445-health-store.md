Code health K04: sqlite store rest and store tests, internal argument lists and long functions (#1445; no schema change; 2026-11-23).
Behaviour-preserving cleanup of the store modules and tests named by the CodeScene inventory (epic #1441). No SQL text, schema, migration body, stored byte or public trait/`pub` signature changed; `archive_before`, `resealable_archives`, `reseal_archives` and the area trait methods keep their signatures because they are published API, so their Excess Number of Function Arguments findings stay. The only new test is a characterisation test of a failed archive (red-first: none, a pure refactor; it passes before and after).

Value structs (all crate-internal): `ClaimKey` (room, owner, path; claim_events.rs) for `do_release`, `do_note_yield_release`, `open_claim_conflict`, `close_claim_conflicts`; `Ended` for `record_ended`; `ArchiveJob` for `archive_before_hooked`; `CheckpointWrite` and `StatusWrite` for `do_put_machine_checkpoint` and `do_put_machine_status`; `View` for `do_mark_shown`. `shown_past` takes a `Seat`; `reseal_label_hooked` takes the key pair as a tuple.

Functions changed: `do_renew_credential` (pure `Stored::renewed_expiry`), `archive_before_hooked` (split into `copy_due_rows` and `tombstone_copied`), `restore_rows` (`seal_restored`), `do_mark_shown` (`contiguous_end`), `Faulty::members` (`overlay_status`), `Faulty::crossing_notices` (`is_crossing_of`, `notice_stage`), the migrations test of the real prototype (three check helpers), `no_unit_test_makes_a_temp_directory_by_hand` (`walk`, `scan_file`, `makes_temp_dir_by_hand`, an allow list), test helpers `row`, `take`, `post`, `post_handoff`, `put`, `issue_with`, `archived_once`, `race_claims`.

| File | Before | After |
| --- | --- | --- |
| store/sqlite/tests/migrations.rs | 8.47 | 8.81 (Large Method resolved; Low Cohesion left: splitting the file would collide with the in-flight migration 44 tests) |
| store/test_dir_tests.rs | 8.85 | 10.0 |
| store/sqlite/credentials.rs | 8.95 | 10.0 |
| store/sqlite/archive.rs | 9.04 | 9.68 (only the public `archive_before` arity remains) |
| store/sqlite/releases.rs | 8.81 | 10.0 |
| store/sqlite/tests/archive.rs | 9.38 | 10.0 |
| store/sqlite/tests/claim_events.rs | 9.38 | 10.0 |
| store/sqlite/tests/concurrency.rs | 9.38 | 10.0 |
| store/sqlite/tests/credentials.rs | 9.38 | 10.0 |
| store/sqlite/tests/label_keys.rs | 9.38 | 10.0 |
| store/memory_spike/faulty.rs | 9.53 | 10.0 |
| store/sqlite/messaging.rs | 9.53 | 10.0 |
| store/sqlite/claim_events.rs, claims.rs, label_keys.rs, machine_checkpoint.rs, integrity.rs | 9.68 | 10.0 |
| store/sqlite/api/claims.rs, members.rs, messages.rs; member/custody/label_rotation.rs | 9.68 | 9.68 (trait signatures are public) |

Left alone: store/area/claims.rs, members.rs, messages.rs (public traits), archive_keys.rs (public functions), the stats files (K01) and store/sqlite/ask* (in-flight #1407).
