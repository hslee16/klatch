//! A backend that declares a feature must keep it (#485). `every_declared_feature_works` runs, for each feature the
//! store declares, the conformance tests of the methods that feature covers, so a declaration the backend cannot
//! back fails here instead of silently losing the feature at run time.
use super::*;

pub fn every_declared_feature_works<S: Store>(make: fn() -> S) {
    for feature in make().features().iter() {
        probe(feature, make);
    }
}

fn probe<S: Store>(feature: Feature, make: fn() -> S) {
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
            members::the_first_parent_noted_for_a_member_is_kept_and_listed_with_it(make);
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
            let s = make();
            s.join("r", "alice", 1).unwrap();
            let id = s
                .enqueue_review(&NewReview::new("r", 1, "private", "why"), 2)
                .unwrap();
            assert!(s.pending_reviews(10).unwrap().iter().any(|q| q.id == id));
        }
        Credentials => {
            credentials::an_issued_credential_is_stored_with_its_hash_clearance_parent_and_expiry(
                make,
            )
        }
    }
}
