//! A backend that declares a feature must keep it (#485). `every_declared_feature_works` runs, for each feature the
//! store declares, the conformance tests of the methods that feature covers, so a declaration the backend cannot
//! back fails here instead of silently losing the feature at run time.
use super::*;

pub fn every_declared_feature_works<S: Store>(make: fn() -> S) {
    for feature in make().features().iter() {
        probe(feature, make);
    }
}

pub(in crate::store) fn probe<S: Store>(feature: Feature, make: fn() -> S) {
    use Feature::*;
    match feature {
        Sessions => {
            members::a_session_is_remembered_as_its_member_and_the_last_note_wins(make);
            members::the_session_that_holds_a_member_name_can_be_looked_up(make);
        }
        MemberNotes => {
            members::a_status_is_stored_per_member_and_room_and_replaced_by_the_next(make);
            members::a_hook_run_is_recorded_per_member_and_event_with_a_count_and_the_size_only(
                make,
            );
            members::hook_bytes_are_totalled_per_event_and_a_session_start_begins_a_new_count(make);
            members::the_first_parent_noted_for_a_member_is_kept_and_listed_with_it(make);
            members::a_park_holds_live_claims_until_the_return_time_and_not_after(make);
            members::a_parked_member_is_listed_with_why_and_return_time_until_unparked(make);
            members::an_unpark_cuts_what_the_park_stretched_back_to_a_normal_lease(make);
            members::an_unpark_of_a_member_that_is_not_parked_changes_nothing(make);
        }
        Mute => members::a_mute_is_kept_per_member_and_room_and_can_be_lifted(make),
        Pruning => {
            messages::pruning_removes_long_gone_members_and_their_bookkeeping_but_keeps_messages_and_recent_members(make)
        }
        WaitPositions => {
            messages::wait_poll_changes_nothing_and_wait_seen_stops_the_next_wake_but_not_read(
                make,
            );
            messages::a_wake_position_is_separate_from_the_wait_position_and_the_read_cursor(make);
        }
        ClaimNotes => {
            claims::a_release_note_travels_to_those_who_saw_the_claim_and_to_the_next_claimant(
                make,
            )
        }
        Anchors => {
            anchors::releasing_an_anchor_releases_its_live_claims_in_every_room_and_only_those(make)
        }
        ClaimLifecycle => {
            claims::claim_changes_reports_a_release_once(make);
            claims::a_cycle_of_three_is_found_at_the_attempt_that_closes_it(make);
        }
        Asks => {
            asks::open_asks_follow_answers_and_claims_and_a_release_leaves_the_ask_open(make)
        }
        Reviews => {
            reviews::queued_items_come_back_oldest_first_and_only_while_pending(make);
            reviews::a_verdict_is_recorded_once(make);
            reviews::the_limit_bounds_one_drain_and_the_same_message_queues_once_per_label(make);
            reviews::the_listing_is_newest_first_and_keeps_resolved_items(make);
            reviews::a_crossing_item_keeps_its_target_requester_and_outcome(make);
            reviews::a_deny_or_a_plain_item_is_never_due_to_be_applied(make);
            reviews::a_crossing_is_told_to_its_requester_once_while_waiting_and_once_for_its_outcome(make);
        }
        Credentials => {
            credentials::an_issued_credential_is_stored_with_its_hash_clearance_parent_and_expiry(
                make,
            )
        }
    }
}
