//! Runs the shared conformance suite against SQLite.
use super::*;
use crate::store::conformance as c;

#[test]
fn a_message_id_finds_its_room_and_an_unknown_id_finds_none() {
    use crate::store::{MessageStore, Seat};
    let s = store();
    let a = s.post(Seat::new("r", "bob"), "one", 1).unwrap();
    let b = s.post(Seat::new("other", "bob"), "two", 2).unwrap();
    assert_eq!(s.message_room(&a.id).unwrap().as_deref(), Some("r"));
    assert_eq!(s.message_room(&b.id).unwrap().as_deref(), Some("other"));
    assert_eq!(s.message_room("no-such-id").unwrap(), None);
}

#[test]
fn messages_post_returns_increasing_ids_and_marks_sender_present() {
    c::messages::post_returns_increasing_ids_and_marks_sender_present(store);
}

#[test]
fn messages_read_excludes_own_messages_and_advances_the_cursor() {
    c::messages::read_excludes_own_messages_and_advances_the_cursor(store);
}

#[test]
fn messages_a_hook_shows_a_message_once_but_read_still_returns_it() {
    c::messages::a_hook_shows_a_message_once_but_read_still_returns_it(store);
}

#[test]
fn messages_a_hook_skips_what_the_member_already_read() {
    c::messages::a_hook_skips_what_the_member_already_read(store);
}

#[test]
fn messages_a_hook_shows_new_messages_after_an_earlier_hook_and_read_returns_all_unread() {
    c::messages::a_hook_shows_new_messages_after_an_earlier_hook_and_read_returns_all_unread(store);
}

#[test]
fn messages_a_hook_excludes_own_messages() {
    c::messages::a_hook_excludes_own_messages(store);
}

#[test]
fn messages_wait_poll_changes_nothing_and_wait_seen_stops_the_next_wake_but_not_read() {
    c::messages::wait_poll_changes_nothing_and_wait_seen_stops_the_next_wake_but_not_read(store);
}

#[test]
fn messages_a_wait_poll_refreshes_the_waiters_presence_like_a_read() {
    c::messages::a_wait_poll_refreshes_the_waiters_presence_like_a_read(store);
}

#[test]
fn messages_wait_seen_only_moves_forward() {
    c::messages::wait_seen_only_moves_forward(store);
}

#[test]
fn messages_wait_poll_skips_what_was_read_and_own_messages() {
    c::messages::wait_poll_skips_what_was_read_and_own_messages(store);
}

#[test]
fn messages_pruning_removes_long_gone_members_and_their_bookkeeping_but_keeps_messages_and_recent_members()
 {
    c::messages::pruning_removes_long_gone_members_and_their_bookkeeping_but_keeps_messages_and_recent_members(store);
}

#[test]
fn messages_peek_does_not_advance_the_cursor() {
    c::messages::peek_does_not_advance_the_cursor(store);
}

#[test]
fn messages_rooms_are_isolated() {
    c::messages::rooms_are_isolated(store);
}

#[test]
fn messages_seq_is_dense_per_room_starting_at_one() {
    c::messages::seq_is_dense_per_room_starting_at_one(store);
}

#[test]
fn messages_message_ids_are_unique_and_well_formed() {
    c::messages::message_ids_are_unique_and_well_formed(store);
}

#[test]
fn messages_read_returns_the_same_id_and_seq_that_post_returned() {
    c::messages::read_returns_the_same_id_and_seq_that_post_returned(store);
}

#[test]
fn messages_read_orders_by_seq_and_the_cursor_follows_seq() {
    c::messages::read_orders_by_seq_and_the_cursor_follows_seq(store);
}

#[test]
fn messages_head_is_zero_for_an_empty_room_and_the_highest_seq_otherwise() {
    c::messages::head_is_zero_for_an_empty_room_and_the_highest_seq_otherwise(store);
}

#[test]
fn messages_tail_returns_messages_after_a_seq_in_order_from_every_sender() {
    c::messages::tail_returns_messages_after_a_seq_in_order_from_every_sender(store);
}

#[test]
fn messages_tail_honours_the_limit_and_the_room() {
    c::messages::tail_honours_the_limit_and_the_room(store);
}

#[test]
fn messages_words_returns_only_the_messages_with_a_prefix_however_deep_they_are() {
    c::messages::words_returns_only_the_messages_with_a_prefix_however_deep_they_are(store);
}

#[test]
fn messages_tail_has_no_side_effects_on_cursors_or_presence() {
    c::messages::tail_has_no_side_effects_on_cursors_or_presence(store);
}

#[test]
fn messages_advance_cursor_moves_forward_only_and_read_follows_it() {
    c::messages::advance_cursor_moves_forward_only_and_read_follows_it(store);
}

#[test]
fn messages_advance_cursor_never_passes_the_head_so_it_cannot_skip_future_messages() {
    c::messages::advance_cursor_never_passes_the_head_so_it_cannot_skip_future_messages(store);
}

#[test]
fn messages_advance_cursor_creates_the_membership_if_needed() {
    c::messages::advance_cursor_creates_the_membership_if_needed(store);
}

#[test]
fn messages_post_as_stores_attribution_and_label_and_read_and_tail_return_them() {
    c::messages::post_as_stores_attribution_and_label_and_read_and_tail_return_them(store);
}

#[test]
fn messages_the_plain_post_is_unattributed_and_public() {
    c::messages::the_plain_post_is_unattributed_and_public(store);
}

#[test]
fn messages_a_wake_position_is_separate_from_the_wait_position_and_the_read_cursor() {
    c::messages::a_wake_position_is_separate_from_the_wait_position_and_the_read_cursor(store);
}

#[test]
fn messages_wake_seen_only_moves_forward_and_a_read_hides_what_it_read_from_a_wake() {
    c::messages::wake_seen_only_moves_forward_and_a_read_hides_what_it_read_from_a_wake(store);
}

#[test]
fn messages_a_wake_poll_refreshes_presence_like_a_wait_poll() {
    c::messages::a_wake_poll_refreshes_presence_like_a_wait_poll(store);
}

#[test]
fn messages_a_message_with_no_handoff_has_none() {
    c::messages::a_message_with_no_handoff_has_none(store);
}

#[test]
fn messages_a_kind_and_payload_are_stored_and_come_back_by_every_read_path() {
    c::messages::a_kind_and_payload_are_stored_and_come_back_by_every_read_path(store);
}

#[test]
fn messages_a_kind_without_a_payload_is_a_label_on_the_message_alone() {
    c::messages::a_kind_without_a_payload_is_a_label_on_the_message_alone(store);
}

#[test]
fn members_join_is_idempotent_and_lists_the_member() {
    c::members::join_is_idempotent_and_lists_the_member(store);
}

#[test]
fn members_rooms_of_is_sorted_and_per_member() {
    c::members::rooms_of_is_sorted_and_per_member(store);
}

#[test]
fn members_a_status_is_stored_per_member_and_room_and_replaced_by_the_next() {
    c::members::a_status_is_stored_per_member_and_room_and_replaced_by_the_next(store);
}

#[test]
fn members_setting_a_status_joins_a_member_who_was_not_there() {
    c::members::setting_a_status_joins_a_member_who_was_not_there(store);
}

#[test]
fn members_hook_bytes_are_totalled_per_event_and_a_session_start_begins_a_new_count() {
    c::members::hook_bytes_are_totalled_per_event_and_a_session_start_begins_a_new_count(store);
}

#[test]
fn members_a_hook_run_is_recorded_per_member_and_event_with_a_count_and_the_size_only() {
    c::members::a_hook_run_is_recorded_per_member_and_event_with_a_count_and_the_size_only(store);
}

#[test]
fn members_a_session_is_remembered_as_its_member_and_the_last_note_wins() {
    c::members::a_session_is_remembered_as_its_member_and_the_last_note_wins(store);
}

#[test]
fn members_the_session_that_holds_a_member_name_can_be_looked_up() {
    c::members::the_session_that_holds_a_member_name_can_be_looked_up(store);
}

#[test]
fn members_room_state_pause_and_blocks_are_remembered_and_audited() {
    c::members::room_state_pause_and_blocks_are_remembered_and_audited(store);
}

#[test]
fn members_a_late_joiner_sees_history() {
    c::members::a_late_joiner_sees_history(store);
}

#[test]
fn members_a_mute_is_kept_per_member_and_room_and_can_be_lifted() {
    c::members::a_mute_is_kept_per_member_and_room_and_can_be_lifted(store);
}

#[test]
fn members_the_first_parent_noted_for_a_member_is_kept_and_listed_with_it() {
    c::members::the_first_parent_noted_for_a_member_is_kept_and_listed_with_it(store);
}

#[test]
fn claims_the_attempt_that_closes_a_two_party_cycle_is_told_it_is_a_deadlock() {
    c::claims::the_attempt_that_closes_a_two_party_cycle_is_told_it_is_a_deadlock(store);
}

#[test]
fn claims_a_plain_conflict_is_not_a_deadlock() {
    c::claims::a_plain_conflict_is_not_a_deadlock(store);
}

#[test]
fn claims_a_wait_older_than_the_window_does_not_make_a_cycle() {
    c::claims::a_wait_older_than_the_window_does_not_make_a_cycle(store);
}

#[test]
fn claims_a_cycle_of_three_is_found_at_the_attempt_that_closes_it() {
    c::claims::a_cycle_of_three_is_found_at_the_attempt_that_closes_it(store);
}

#[test]
fn claims_a_satisfied_wait_no_longer_counts() {
    c::claims::a_satisfied_wait_no_longer_counts(store);
}

#[test]
fn claims_waits_in_one_room_do_not_make_a_cycle_in_another() {
    c::claims::waits_in_one_room_do_not_make_a_cycle_in_another(store);
}

#[test]
fn claims_a_shared_claim_collides_with_every_scope_in_both_directions() {
    c::claims::a_shared_claim_collides_with_every_scope_in_both_directions(store);
}

#[test]
fn claims_an_empty_scope_behaves_as_claims_always_did_and_the_listing_shows_each_scope() {
    c::claims::an_empty_scope_behaves_as_claims_always_did_and_the_listing_shows_each_scope(store);
}

#[test]
fn claims_a_lapse_after_a_reclaim_shows_no_stale_release_note() {
    c::claims::a_lapse_after_a_reclaim_shows_no_stale_release_note(store);
}

#[test]
fn claims_a_note_of_another_owner_survives_a_reclaim_by_the_first() {
    c::claims::a_note_of_another_owner_survives_a_reclaim_by_the_first(store);
}

#[test]
fn claims_a_release_note_travels_to_those_who_saw_the_claim_and_to_the_next_claimant() {
    c::claims::a_release_note_travels_to_those_who_saw_the_claim_and_to_the_next_claimant(store);
}

#[test]
fn claims_removing_a_member_takes_it_out_of_the_room_and_releases_its_claims_there() {
    c::claims::removing_a_member_takes_it_out_of_the_room_and_releases_its_claims_there(store);
}

#[test]
fn claims_claims_do_not_conflict_across_rooms() {
    c::claims::claims_do_not_conflict_across_rooms(store);
}

#[test]
fn claims_an_owner_can_reclaim_and_replace_their_own_path() {
    c::claims::an_owner_can_reclaim_and_replace_their_own_path(store);
}

#[test]
fn claims_expired_claims_free_the_path_and_are_not_listed() {
    c::claims::expired_claims_free_the_path_and_are_not_listed(store);
}

#[test]
fn claims_claims_are_ordered_by_path() {
    c::claims::claims_are_ordered_by_path(store);
}

#[test]
fn claims_claim_changes_first_call_shows_others_claims_then_nothing() {
    c::claims::claim_changes_first_call_shows_others_claims_then_nothing(store);
}

#[test]
fn claims_claim_changes_ignores_the_passing_of_time() {
    c::claims::claim_changes_ignores_the_passing_of_time(store);
}

#[test]
fn claims_claim_changes_reports_only_the_new_claim() {
    c::claims::claim_changes_reports_only_the_new_claim(store);
}

#[test]
fn claims_claim_changes_reports_a_renewed_claim_again() {
    c::claims::claim_changes_reports_a_renewed_claim_again(store);
}

#[test]
fn claims_claim_changes_reports_a_release_once() {
    c::claims::claim_changes_reports_a_release_once(store);
}

#[test]
fn claims_claim_changes_are_remembered_per_member_and_per_room() {
    c::claims::claim_changes_are_remembered_per_member_and_per_room(store);
}

#[test]
fn claims_claim_changes_does_not_hold_the_writer_lock_after_it_returns() {
    c::claims::claim_changes_does_not_hold_the_writer_lock_after_it_returns(store);
}

#[test]
fn asks_open_asks_follow_answers_and_claims_and_a_release_leaves_the_ask_open() {
    c::asks::open_asks_follow_answers_and_claims_and_a_release_leaves_the_ask_open(store);
}

#[test]
fn asks_an_ask_and_its_answer_are_found_however_deep_they_are_in_the_room() {
    c::asks::an_ask_and_its_answer_are_found_however_deep_they_are_in_the_room(store);
}

#[test]
fn asks_a_reply_or_a_close_ends_an_open_ask() {
    c::asks::a_reply_or_a_close_ends_an_open_ask(store);
}

#[test]
fn asks_stale_asks_report_my_unclaimed_old_asks_once() {
    c::asks::stale_asks_report_my_unclaimed_old_asks_once(store);
}

#[test]
fn asks_a_claimed_or_answered_ask_is_not_stale() {
    c::asks::a_claimed_or_answered_ask_is_not_stale(store);
}

#[test]
fn asks_an_asker_is_told_once_that_nobody_here_can_do_its_ask() {
    c::asks::an_asker_is_told_once_that_nobody_here_can_do_its_ask(store);
}

#[test]
fn asks_a_busy_helper_means_someone_capable_exists_so_no_escalation() {
    c::asks::a_busy_helper_means_someone_capable_exists_so_no_escalation(store);
}

#[test]
fn asks_a_moved_ask_is_alive_while_its_dm_has_news_and_shows_the_name() {
    c::asks::a_moved_ask_is_alive_while_its_dm_has_news_and_shows_the_name(store);
}
#[test]
fn asks_a_working_room_is_listed_with_its_members_and_activity_and_attached_to_its_ask() {
    c::asks::a_working_room_is_listed_with_its_members_and_activity_and_attached_to_its_ask(store);
}

#[test]
fn asks_an_ask_waiting_on_its_asker_or_with_a_busy_room_is_not_reported_stale() {
    c::asks::an_ask_waiting_on_its_asker_or_with_a_busy_room_is_not_reported_stale(store);
}

#[test]
fn asks_a_lapsed_ask_claim_is_reported_to_the_asker_once_with_the_holder() {
    c::asks::a_lapsed_ask_claim_is_reported_to_the_asker_once_with_the_holder(store);
}

#[test]
fn asks_releasing_a_members_claims_frees_every_room_and_tells_the_asker() {
    c::asks::releasing_a_members_claims_frees_every_room_and_tells_the_asker(store);
}

#[test]
fn asks_a_release_an_answer_or_a_fresh_claim_is_not_a_lapse() {
    c::asks::a_release_an_answer_or_a_fresh_claim_is_not_a_lapse(store);
}

#[test]
fn asks_a_members_presence_in_a_working_room_is_not_activity_in_it() {
    c::asks::a_members_presence_in_a_working_room_is_not_activity_in_it(store);
}

#[test]
fn asks_claim_changes_reports_a_lapsed_claim_as_released_once() {
    c::asks::claim_changes_reports_a_lapsed_claim_as_released_once(store);
}

#[test]
fn asks_claim_changes_never_reports_a_claim_that_lapsed_before_it_was_seen() {
    c::asks::claim_changes_never_reports_a_claim_that_lapsed_before_it_was_seen(store);
}

#[test]
fn anchors_releasing_an_anchor_releases_its_live_claims_in_every_room_and_only_those() {
    c::anchors::releasing_an_anchor_releases_its_live_claims_in_every_room_and_only_those(store);
}

#[test]
fn anchors_an_expired_claim_is_cleared_but_not_reported_as_released() {
    c::anchors::an_expired_claim_is_cleared_but_not_reported_as_released(store);
}

#[test]
fn anchors_a_released_path_can_be_claimed_by_someone_else_at_once() {
    c::anchors::a_released_path_can_be_claimed_by_someone_else_at_once(store);
}

#[test]
fn anchors_the_anchors_in_use_are_the_distinct_anchors_of_live_claims() {
    c::anchors::the_anchors_in_use_are_the_distinct_anchors_of_live_claims(store);
}

#[test]
fn policy_a_policy_value_is_set_read_replaced_and_cleared() {
    c::policy::a_policy_value_is_set_read_replaced_and_cleared(store);
}

#[test]
fn policy_policy_entries_are_listed_by_prefix_in_key_order() {
    c::policy::policy_entries_are_listed_by_prefix_in_key_order(store);
}

#[test]
fn policy_a_prefix_with_wildcard_characters_matches_literally() {
    c::policy::a_prefix_with_wildcard_characters_matches_literally(store);
}

#[test]
fn policy_refusals_are_kept_newest_first_with_the_rule_and_no_text() {
    c::policy::refusals_are_kept_newest_first_with_the_rule_and_no_text(store);
}

#[test]
fn policy_one_members_refusals_are_listed_newest_first_and_the_limit_counts_only_theirs() {
    c::policy::one_members_refusals_are_listed_newest_first_and_the_limit_counts_only_theirs(store);
}

#[test]
fn policy_the_rules_a_member_was_refused_under_since_a_time_are_counted_per_member() {
    c::policy::the_rules_a_member_was_refused_under_since_a_time_are_counted_per_member(store);
}

#[test]
fn policy_the_audit_page_filters_before_the_limit_leaves_out_the_note_and_counts_what_it_hid() {
    c::policy::the_audit_page_filters_before_the_limit_leaves_out_the_note_and_counts_what_it_hid(
        store,
    );
}

#[test]
fn credentials_an_issued_credential_is_stored_with_its_hash_clearance_parent_and_expiry() {
    c::credentials::an_issued_credential_is_stored_with_its_hash_clearance_parent_and_expiry(store);
}

#[test]
fn credentials_revoking_works_once_and_marks_the_record_without_deleting_it() {
    c::credentials::revoking_works_once_and_marks_the_record_without_deleting_it(store);
}

#[test]
fn credentials_the_list_has_every_credential_by_member() {
    c::credentials::the_list_has_every_credential_by_member(store);
}

#[test]
fn members_a_park_holds_live_claims_until_the_return_time_and_not_after() {
    c::members::a_park_holds_live_claims_until_the_return_time_and_not_after(store);
}

#[test]
fn members_a_parked_member_is_listed_with_why_and_return_time_until_unparked() {
    c::members::a_parked_member_is_listed_with_why_and_return_time_until_unparked(store);
}

#[test]
fn members_an_unpark_cuts_what_the_park_stretched_back_to_a_normal_lease() {
    c::members::an_unpark_cuts_what_the_park_stretched_back_to_a_normal_lease(store);
}

#[test]
fn members_an_unpark_of_a_member_that_is_not_parked_changes_nothing() {
    c::members::an_unpark_of_a_member_that_is_not_parked_changes_nothing(store);
}

#[test]
fn reviews_queued_items_come_back_oldest_first_and_only_while_pending() {
    c::reviews::queued_items_come_back_oldest_first_and_only_while_pending(store);
}

#[test]
fn reviews_a_verdict_is_recorded_once() {
    c::reviews::a_verdict_is_recorded_once(store);
}

#[test]
fn reviews_the_limit_bounds_one_drain_and_the_same_message_queues_once_per_label() {
    c::reviews::the_limit_bounds_one_drain_and_the_same_message_queues_once_per_label(store);
}

#[test]
fn reviews_the_listing_is_newest_first_and_keeps_resolved_items() {
    c::reviews::the_listing_is_newest_first_and_keeps_resolved_items(store);
}

#[test]
fn reviews_a_crossing_item_keeps_its_target_requester_and_outcome() {
    c::reviews::a_crossing_item_keeps_its_target_requester_and_outcome(store);
}

#[test]
fn reviews_a_deny_or_a_plain_item_is_never_due_to_be_applied() {
    c::reviews::a_deny_or_a_plain_item_is_never_due_to_be_applied(store);
}

#[test]
fn reviews_a_crossing_is_told_to_its_requester_once_while_waiting_and_once_for_its_outcome() {
    c::reviews::a_crossing_is_told_to_its_requester_once_while_waiting_and_once_for_its_outcome(
        store,
    );
}

#[test]
fn sqlite_declares_every_feature_and_keeps_each_one() {
    assert!(store().features().missing().is_empty());
    c::features::every_declared_feature_works(store);
}

#[test]
fn reviews_pending_pages_by_cursor_and_waiting_crossings_are_counted_per_requester() {
    c::reviews::pending_pages_by_cursor_and_waiting_crossings_are_counted_per_requester(store);
}
