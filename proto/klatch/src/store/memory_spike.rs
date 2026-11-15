//! A second `Store`, in memory, written only to find out what the trait demands (#73). It implements the eleven
//! methods the trait requires and none of the optional ones, so the conformance suite shows which behaviours need
//! more than the required eleven. It is test-only and is not a supported backend.

use super::*;
use crate::domain::{Error, overlaps};
use std::cell::RefCell;
use std::collections::BTreeMap;

#[derive(Default)]
struct Room {
    messages: Vec<Message>,
    members: BTreeMap<String, (i64, i64)>, // name -> (last_seen, last_seq)
    claims: Vec<(Claim, i64)>,             // claim, expires
}

#[derive(Default)]
pub struct MemoryStore {
    rooms: RefCell<BTreeMap<String, Room>>,
    next: RefCell<u64>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Store for MemoryStore {
    fn join(&self, room: &str, member: &str, now: i64) -> Result<()> {
        let mut rooms = self.rooms.borrow_mut();
        let r = rooms.entry(room.to_string()).or_default();
        let e = r.members.entry(member.to_string()).or_insert((now, 0));
        e.0 = now;
        Ok(())
    }

    fn post_as(&self, m: &NewMessage, now: i64) -> Result<Posted> {
        self.join(m.room, m.sender, now)?;
        let mut rooms = self.rooms.borrow_mut();
        let r = rooms.get_mut(m.room).unwrap();
        let seq = r.messages.len() as i64 + 1;
        let mut n = self.next.borrow_mut();
        *n += 1;
        let id = format!("{:026}", *n);
        r.messages.push(Message {
            id: id.clone(),
            seq,
            sender: m.sender.to_string(),
            attribution: m.attribution,
            label: m.label.to_string(),
            body: m.body.to_string(),
            ts: now,
            kind: m.kind.map(String::from),
            payload: m.payload.map(String::from),
        });
        Ok(Posted { id, seq })
    }

    fn read(&self, at: Seat, advance: bool, now: i64) -> Result<Vec<Message>> {
        let mut rooms = self.rooms.borrow_mut();
        let r = rooms.entry(at.room.to_string()).or_default();
        let e = r.members.entry(at.member.to_string()).or_insert((now, 0));
        e.0 = now;
        let out: Vec<Message> = r
            .messages
            .iter()
            .filter(|m| m.seq > e.1 && m.sender != at.member)
            .cloned()
            .collect();
        if advance {
            e.1 = r.messages.len() as i64;
        }
        Ok(out)
    }

    fn members(&self, room: &str) -> Result<Vec<MemberInfo>> {
        Ok(self
            .rooms
            .borrow()
            .get(room)
            .map(|r| {
                r.members
                    .iter()
                    .map(|(n, (seen, _))| MemberInfo {
                        name: n.clone(),
                        joined: 0,
                        last_seen: *seen,
                        status: None,
                        parent: None,
                        parked: None,
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    fn claim(&self, q: &ClaimRequest, now: i64) -> Result<()> {
        let mut rooms = self.rooms.borrow_mut();
        let r = rooms.entry(q.room.to_string()).or_default();
        r.claims.retain(|(_, exp)| *exp > now);
        let scope = if q.scope.is_empty() { "" } else { q.scope };
        for (c, exp) in &r.claims {
            let same_scope = c.scope == scope || c.scope == "*" || scope == "*";
            if c.owner != q.owner && same_scope && overlaps(&c.path, q.path) {
                return Err(Error::Conflict {
                    path: c.path.clone(),
                    owner: c.owner.clone(),
                    secs_left: exp - now,
                    deadlock: false,
                });
            }
        }
        r.claims
            .retain(|(c, _)| !(c.owner == q.owner && c.path == q.path));
        r.claims.push((
            Claim {
                path: q.path.to_string(),
                owner: q.owner.to_string(),
                reason: q.reason.to_string(),
                expires: now + q.ttl,
                scope: scope.to_string(),
            },
            now + q.ttl,
        ));
        Ok(())
    }

    fn release(&self, room: &str, owner: &str, path: &str) -> Result<()> {
        if let Some(r) = self.rooms.borrow_mut().get_mut(room) {
            r.claims
                .retain(|(c, _)| !(c.owner == owner && c.path == path));
        }
        Ok(())
    }

    fn claims(&self, room: &str, now: i64) -> Result<Vec<Claim>> {
        Ok(self
            .rooms
            .borrow()
            .get(room)
            .map(|r| {
                r.claims
                    .iter()
                    .filter(|(_, exp)| *exp > now)
                    .map(|(c, _)| c.clone())
                    .collect()
            })
            .unwrap_or_default())
    }

    fn rooms_of(&self, member: &str) -> Result<Vec<String>> {
        Ok(self
            .rooms
            .borrow()
            .iter()
            .filter(|(_, r)| r.members.contains_key(member))
            .map(|(n, _)| n.clone())
            .collect())
    }

    fn tail(&self, room: &str, after_seq: i64, limit: usize) -> Result<Vec<Message>> {
        Ok(self
            .rooms
            .borrow()
            .get(room)
            .map(|r| {
                r.messages
                    .iter()
                    .filter(|m| m.seq > after_seq)
                    .take(limit)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default())
    }

    fn head(&self, room: &str) -> Result<i64> {
        Ok(self
            .rooms
            .borrow()
            .get(room)
            .map(|r| r.messages.len() as i64)
            .unwrap_or(0))
    }

    fn advance_cursor(&self, at: Seat, seq: i64, now: i64) -> Result<()> {
        let mut rooms = self.rooms.borrow_mut();
        let r = rooms.entry(at.room.to_string()).or_default();
        let head = r.messages.len() as i64;
        let e = r.members.entry(at.member.to_string()).or_insert((now, 0));
        e.0 = now;
        e.1 = e.1.max(seq.min(head)).max(0);
        Ok(())
    }
}

/// Runs every conformance test on the in-memory store and prints which pass. Not an assertion on the count: the
/// point is the list. Run with `cargo test --lib memory_spike -- --ignored --nocapture`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::conformance as c;

    macro_rules! run {
        ($($m:ident :: $f:ident),* $(,)?) => {{
            let mut pass = Vec::new();
            let mut fail = Vec::new();
            $(
                let r = std::panic::catch_unwind(|| c::$m::$f(MemoryStore::new));
                if r.is_ok() { pass.push(stringify!($m::$f)); } else { fail.push(stringify!($m::$f)); }
            )*
            (pass, fail)
        }};
    }

    /// The core behaviours pass on a store written from the trait alone, which shows the suite and the trait do not
    /// depend on SQLite.
    #[test]
    fn the_core_behaviours_hold_on_a_second_backend() {
        c::messages::post_returns_increasing_ids_and_marks_sender_present(MemoryStore::new);
        c::messages::read_excludes_own_messages_and_advances_the_cursor(MemoryStore::new);
        c::members::join_is_idempotent_and_lists_the_member(MemoryStore::new);
    }

    #[test]
    #[ignore]
    fn which_conformance_tests_pass_on_the_required_methods_alone() {
        let (pass, fail) = run!(messages::post_returns_increasing_ids_and_marks_sender_present, messages::read_excludes_own_messages_and_advances_the_cursor, messages::a_hook_shows_a_message_once_but_read_still_returns_it, messages::a_hook_skips_what_the_member_already_read, messages::a_hook_shows_new_messages_after_an_earlier_hook_and_read_returns_all_unread, messages::a_hook_excludes_own_messages, messages::wait_poll_changes_nothing_and_wait_seen_stops_the_next_wake_but_not_read, messages::a_wait_poll_refreshes_the_waiters_presence_like_a_read, messages::wait_seen_only_moves_forward, messages::wait_poll_skips_what_was_read_and_own_messages, messages::pruning_removes_long_gone_members_and_their_bookkeeping_but_keeps_messages_and_recent_members, messages::peek_does_not_advance_the_cursor, messages::rooms_are_isolated, messages::seq_is_dense_per_room_starting_at_one, messages::message_ids_are_unique_and_well_formed, messages::read_returns_the_same_id_and_seq_that_post_returned, messages::read_orders_by_seq_and_the_cursor_follows_seq, messages::head_is_zero_for_an_empty_room_and_the_highest_seq_otherwise, messages::tail_returns_messages_after_a_seq_in_order_from_every_sender, messages::tail_honours_the_limit_and_the_room, messages::tail_has_no_side_effects_on_cursors_or_presence, messages::advance_cursor_moves_forward_only_and_read_follows_it, messages::advance_cursor_never_passes_the_head_so_it_cannot_skip_future_messages, messages::advance_cursor_creates_the_membership_if_needed, messages::post_as_stores_attribution_and_label_and_read_and_tail_return_them, messages::the_plain_post_is_unattributed_and_public, messages::a_wake_position_is_separate_from_the_wait_position_and_the_read_cursor, messages::wake_seen_only_moves_forward_and_a_read_hides_what_it_read_from_a_wake, messages::a_wake_poll_refreshes_presence_like_a_wait_poll, messages::a_message_with_no_handoff_has_none, messages::a_kind_and_payload_are_stored_and_come_back_by_every_read_path, messages::a_kind_without_a_payload_is_a_label_on_the_message_alone, members::join_is_idempotent_and_lists_the_member, members::rooms_of_is_sorted_and_per_member, members::a_status_is_stored_per_member_and_room_and_replaced_by_the_next, members::setting_a_status_joins_a_member_who_was_not_there, members::a_hook_run_is_recorded_per_member_and_event_with_a_count_and_the_size_only, members::a_session_is_remembered_as_its_member_and_the_last_note_wins, members::the_session_that_holds_a_member_name_can_be_looked_up, members::room_state_pause_and_blocks_are_remembered_and_audited, members::a_late_joiner_sees_history, members::a_mute_is_kept_per_member_and_room_and_can_be_lifted, members::the_first_parent_noted_for_a_member_is_kept_and_listed_with_it, claims::the_attempt_that_closes_a_two_party_cycle_is_told_it_is_a_deadlock, claims::a_plain_conflict_is_not_a_deadlock, claims::a_wait_older_than_the_window_does_not_make_a_cycle, claims::a_cycle_of_three_is_found_at_the_attempt_that_closes_it, claims::a_satisfied_wait_no_longer_counts, claims::waits_in_one_room_do_not_make_a_cycle_in_another, claims::a_shared_claim_collides_with_every_scope_in_both_directions, claims::an_empty_scope_behaves_as_claims_always_did_and_the_listing_shows_each_scope, claims::a_release_note_travels_to_those_who_saw_the_claim_and_to_the_next_claimant, claims::removing_a_member_takes_it_out_of_the_room_and_releases_its_claims_there, claims::claims_do_not_conflict_across_rooms, claims::an_owner_can_reclaim_and_replace_their_own_path, claims::expired_claims_free_the_path_and_are_not_listed, claims::claims_are_ordered_by_path, claims::claim_changes_first_call_shows_others_claims_then_nothing, claims::claim_changes_ignores_the_passing_of_time, claims::claim_changes_reports_only_the_new_claim, claims::claim_changes_reports_a_renewed_claim_again, claims::claim_changes_reports_a_release_once, claims::claim_changes_are_remembered_per_member_and_per_room, claims::claim_changes_does_not_hold_the_writer_lock_after_it_returns, asks::open_asks_follow_answers_and_claims_and_a_release_leaves_the_ask_open, asks::a_reply_or_a_close_ends_an_open_ask, asks::stale_asks_report_my_unclaimed_old_asks_once, asks::a_claimed_or_answered_ask_is_not_stale, asks::an_asker_is_told_once_that_nobody_here_can_do_its_ask, asks::a_busy_helper_means_someone_capable_exists_so_no_escalation, asks::a_working_room_is_listed_with_its_members_and_activity_and_attached_to_its_ask, asks::an_ask_waiting_on_its_asker_or_with_a_busy_room_is_not_reported_stale, asks::a_lapsed_ask_claim_is_reported_to_the_asker_once_with_the_holder, asks::releasing_a_members_claims_frees_every_room_and_tells_the_asker, asks::a_release_an_answer_or_a_fresh_claim_is_not_a_lapse, asks::a_members_presence_in_a_working_room_is_not_activity_in_it, asks::claim_changes_reports_a_lapsed_claim_as_released_once, asks::claim_changes_never_reports_a_claim_that_lapsed_before_it_was_seen, anchors::releasing_an_anchor_releases_its_live_claims_in_every_room_and_only_those, anchors::an_expired_claim_is_cleared_but_not_reported_as_released, anchors::a_released_path_can_be_claimed_by_someone_else_at_once, anchors::the_anchors_in_use_are_the_distinct_anchors_of_live_claims, policy::a_policy_value_is_set_read_replaced_and_cleared, policy::policy_entries_are_listed_by_prefix_in_key_order, policy::a_prefix_with_wildcard_characters_matches_literally, policy::refusals_are_kept_newest_first_with_the_rule_and_no_text, policy::the_rules_a_member_was_refused_under_since_a_time_are_counted_per_member, credentials::an_issued_credential_is_stored_with_its_hash_clearance_parent_and_expiry, credentials::revoking_works_once_and_marks_the_record_without_deleting_it, credentials::the_list_has_every_credential_by_member);
        println!("PASS {}", pass.len());
        println!("FAIL {}: {}", fail.len(), fail.join(", "));
    }
}
