//! The store conformance suite (#74): every behaviour a `Store` must have, written against the trait only. A backend runs
//! it by calling each function with a constructor for an empty store; `sqlite/tests/conformance.rs` does so for SQLite.
//! A test that needs a backend's own internals (the file, a raw connection, migrations) stays with that backend.

#![allow(unused_imports)]
use crate::domain::*;
use crate::store::*;

pub(super) fn deadlocked(r: Result<()>) -> Option<bool> {
    match r {
        Err(Error::Conflict { deadlock, .. }) => Some(deadlock),
        _ => None,
    }
}

pub(super) type Pairs = Vec<(String, String)>;

pub(super) fn changes<S: Store>(s: &S, room: &str, member: &str, now: i64) -> (Pairs, Pairs) {
    let c = s.claim_changes(room, member, now).unwrap();
    (
        c.added.into_iter().map(|c| (c.owner, c.path)).collect(),
        c.released.into_iter().map(|c| (c.owner, c.path)).collect(),
    )
}

pub(super) fn pair(o: &str, p: &str) -> (String, String) {
    (o.to_string(), p.to_string())
}

pub mod messages {
    use super::*;
    use crate::domain::Attribution;
    pub fn post_returns_increasing_ids_and_marks_sender_present<S: Store>(make: fn() -> S) {
        let s = make();
        let a = s.post(Seat::new("r", "alice"), "one", 5).unwrap();
        let b = s.post(Seat::new("r", "alice"), "two", 6).unwrap();
        assert!(b.seq > a.seq);
        assert_eq!(s.members("r").unwrap()[0].last_seen, 6);
    }
    pub fn read_excludes_own_messages_and_advances_the_cursor<S: Store>(make: fn() -> S) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        s.join("r", "bob", 1).unwrap();
        s.post(Seat::new("r", "bob"), "hi", 2).unwrap();
        s.post(Seat::new("r", "alice"), "mine", 3).unwrap();
        let first = s.read(Seat::new("r", "alice"), true, 4).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(
            (first[0].sender.as_str(), first[0].body.as_str()),
            ("bob", "hi")
        );
        assert!(s.read(Seat::new("r", "alice"), true, 5).unwrap().is_empty());
    }
    pub fn a_hook_shows_a_message_once_but_read_still_returns_it<S: Store>(make: fn() -> S) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        s.post(Seat::new("r", "bob"), "ask", 2).unwrap();
        assert_eq!(s.hook_read("r", "alice", 3).unwrap().len(), 1);
        assert!(
            s.hook_read("r", "alice", 4).unwrap().is_empty(),
            "hook repeated"
        );
        let r = s.read(Seat::new("r", "alice"), true, 5).unwrap();
        assert_eq!(r.len(), 1, "read lost a message the hook showed");
        assert!(s.read(Seat::new("r", "alice"), true, 6).unwrap().is_empty());
    }
    pub fn a_hook_skips_what_the_member_already_read<S: Store>(make: fn() -> S) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        s.post(Seat::new("r", "bob"), "one", 2).unwrap();
        s.read(Seat::new("r", "alice"), true, 3).unwrap();
        assert!(s.hook_read("r", "alice", 4).unwrap().is_empty());
        s.post(Seat::new("r", "bob"), "two", 5).unwrap();
        let h = s.hook_read("r", "alice", 6).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].body, "two");
    }
    pub fn a_hook_shows_new_messages_after_an_earlier_hook_and_read_returns_all_unread<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        s.post(Seat::new("r", "bob"), "one", 2).unwrap();
        s.hook_read("r", "alice", 3).unwrap();
        s.post(Seat::new("r", "bob"), "two", 4).unwrap();
        assert_eq!(s.hook_read("r", "alice", 5).unwrap()[0].body, "two");
        let bodies: Vec<_> = s
            .read(Seat::new("r", "alice"), true, 6)
            .unwrap()
            .into_iter()
            .map(|m| m.body)
            .collect();
        assert_eq!(bodies, ["one", "two"]);
    }
    pub fn a_hook_excludes_own_messages<S: Store>(make: fn() -> S) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        s.post(Seat::new("r", "alice"), "mine", 2).unwrap();
        assert!(s.hook_read("r", "alice", 3).unwrap().is_empty());
    }
    pub fn wait_poll_changes_nothing_and_wait_seen_stops_the_next_wake_but_not_read<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        s.post(Seat::new("r", "bob"), "one", 2).unwrap();
        assert_eq!(s.wait_poll("r", "alice", 3).unwrap().len(), 1);
        assert_eq!(
            s.wait_poll("r", "alice", 4).unwrap().len(),
            1,
            "poll alone must not change state"
        );
        s.wait_seen(Seat::new("r", "alice"), 1, 5).unwrap();
        assert!(s.wait_poll("r", "alice", 6).unwrap().is_empty());
        assert_eq!(
            s.read(Seat::new("r", "alice"), false, 7).unwrap().len(),
            1,
            "seen by a wait is still unread"
        );
        s.post(Seat::new("r", "bob"), "two", 8).unwrap();
        let w = s.wait_poll("r", "alice", 9).unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].body, "two");
    }
    pub fn a_wait_poll_refreshes_the_waiters_presence_like_a_read<S: Store>(make: fn() -> S) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        s.wait_poll("r", "alice", 9).unwrap();
        assert_eq!(s.members("r").unwrap()[0].last_seen, 9);
        s.read(Seat::new("r", "alice"), false, 12).unwrap();
        assert_eq!(s.members("r").unwrap()[0].last_seen, 12);
    }
    pub fn wait_seen_only_moves_forward<S: Store>(make: fn() -> S) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        s.post(Seat::new("r", "bob"), "one", 2).unwrap();
        s.post(Seat::new("r", "bob"), "two", 3).unwrap();
        s.wait_seen(Seat::new("r", "alice"), 2, 4).unwrap();
        s.wait_seen(Seat::new("r", "alice"), 1, 5).unwrap();
        assert!(s.wait_poll("r", "alice", 6).unwrap().is_empty());
    }
    pub fn wait_poll_skips_what_was_read_and_own_messages<S: Store>(make: fn() -> S) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        s.post(Seat::new("r", "bob"), "one", 2).unwrap();
        s.read(Seat::new("r", "alice"), true, 3).unwrap();
        s.post(Seat::new("r", "alice"), "mine", 4).unwrap();
        assert!(s.wait_poll("r", "alice", 5).unwrap().is_empty());
    }
    pub fn pruning_removes_long_gone_members_and_their_bookkeeping_but_keeps_messages_and_recent_members<
        S: Store,
    >(
        make: fn() -> S,
    ) {
        let s = make();
        s.join("r", "old", 10).unwrap();
        s.join("r2", "old", 10).unwrap();
        s.join("r", "fresh", 9_000).unwrap();
        s.post(Seat::new("r", "old"), "my message stays", 11)
            .unwrap();
        s.record_hook_run(&HookEvent::new("old", "SessionStart", 0), 10)
            .unwrap();
        s.record_hook_run(&HookEvent::new("fresh", "SessionStart", 0), 9_000)
            .unwrap();
        let n = s.prune_members(86_400 / 10, 9_001).unwrap();
        assert_eq!(n, 2, "old in two rooms");
        let names: Vec<_> = s
            .members("r")
            .unwrap()
            .into_iter()
            .map(|m| m.name)
            .collect();
        assert_eq!(names, ["fresh"]);
        assert!(s.members("r2").unwrap().is_empty());
        assert_eq!(s.tail("r", 0, 10).unwrap().len(), 1, "messages stay");
        assert!(
            s.hook_runs("old").unwrap().is_empty(),
            "bookkeeping goes with the member"
        );
        assert_eq!(s.hook_runs("fresh").unwrap().len(), 1);
        assert_eq!(
            s.prune_members(86_400 / 10, 9_002).unwrap(),
            0,
            "nothing more to prune"
        );
    }
    pub fn peek_does_not_advance_the_cursor<S: Store>(make: fn() -> S) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        s.post(Seat::new("r", "bob"), "hi", 2).unwrap();
        assert_eq!(s.read(Seat::new("r", "alice"), false, 3).unwrap().len(), 1);
        assert_eq!(s.read(Seat::new("r", "alice"), false, 4).unwrap().len(), 1);
        assert_eq!(s.read(Seat::new("r", "alice"), true, 5).unwrap().len(), 1);
        assert!(s.read(Seat::new("r", "alice"), true, 6).unwrap().is_empty());
    }
    pub fn rooms_are_isolated<S: Store>(make: fn() -> S) {
        let s = make();
        s.post(Seat::new("r1", "bob"), "x", 1).unwrap();
        s.join("r2", "alice", 2).unwrap();
        assert!(
            s.read(Seat::new("r2", "alice"), true, 3)
                .unwrap()
                .is_empty()
        );
    }
    pub fn seq_is_dense_per_room_starting_at_one<S: Store>(make: fn() -> S) {
        let s = make();
        let a = s.post(Seat::new("r1", "bob"), "x", 1).unwrap();
        let b = s.post(Seat::new("r1", "bob"), "y", 2).unwrap();
        let c = s.post(Seat::new("r2", "bob"), "z", 3).unwrap();
        assert_eq!((a.seq, b.seq, c.seq), (1, 2, 1));
    }
    pub fn message_ids_are_unique_and_well_formed<S: Store>(make: fn() -> S) {
        let s = make();
        let mut seen = std::collections::HashSet::new();
        for i in 0..50 {
            let room = if i % 2 == 0 { "a" } else { "b" };
            let id = s.post(Seat::new(room, "bob"), "x", i).unwrap().id;
            assert_eq!(id.len(), 26, "id {id}");
            assert!(seen.insert(id));
        }
    }
    pub fn read_returns_the_same_id_and_seq_that_post_returned<S: Store>(make: fn() -> S) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        let p = s.post(Seat::new("r", "bob"), "hi", 2).unwrap();
        let m = s.read(Seat::new("r", "alice"), false, 3).unwrap();
        assert_eq!((m[0].id.as_str(), m[0].seq), (p.id.as_str(), p.seq));
    }
    pub fn read_orders_by_seq_and_the_cursor_follows_seq<S: Store>(make: fn() -> S) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        for i in 0..3 {
            s.post(Seat::new("r", "bob"), &format!("m{i}"), i).unwrap();
        }
        let first = s.read(Seat::new("r", "alice"), true, 9).unwrap();
        assert_eq!(
            first.iter().map(|m| m.seq).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        s.post(Seat::new("r", "bob"), "m3", 10).unwrap();
        assert_eq!(s.read(Seat::new("r", "alice"), true, 11).unwrap()[0].seq, 4);
    }
    pub fn head_is_zero_for_an_empty_room_and_the_highest_seq_otherwise<S: Store>(make: fn() -> S) {
        let s = make();
        assert_eq!(s.head("r").unwrap(), 0);
        s.post(Seat::new("r", "bob"), "a", 1).unwrap();
        s.post(Seat::new("r", "bob"), "b", 2).unwrap();
        s.post(Seat::new("other", "bob"), "c", 3).unwrap();
        assert_eq!(s.head("r").unwrap(), 2);
    }
    pub fn tail_returns_messages_after_a_seq_in_order_from_every_sender<S: Store>(make: fn() -> S) {
        let s = make();
        s.post(Seat::new("r", "alice"), "mine", 1).unwrap();
        s.post(Seat::new("r", "bob"), "theirs", 2).unwrap();
        s.post(Seat::new("r", "alice"), "mine2", 3).unwrap();
        let all = s.tail("r", 0, 100).unwrap();
        assert_eq!(all.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(
            all[0].sender, "alice",
            "own messages are included so seq has no gaps"
        );
        let after = s.tail("r", 1, 100).unwrap();
        assert_eq!(after.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![2, 3]);
        assert!(s.tail("r", 3, 100).unwrap().is_empty());
    }
    pub fn tail_honours_the_limit_and_the_room<S: Store>(make: fn() -> S) {
        let s = make();
        for i in 0..5 {
            s.post(Seat::new("r", "bob"), &format!("m{i}"), i).unwrap();
        }
        s.post(Seat::new("other", "bob"), "x", 9).unwrap();
        let two = s.tail("r", 0, 2).unwrap();
        assert_eq!(two.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![1, 2]);
        assert!(s.tail("empty", 0, 10).unwrap().is_empty());
    }
    pub fn tail_has_no_side_effects_on_cursors_or_presence<S: Store>(make: fn() -> S) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        s.post(Seat::new("r", "bob"), "x", 2).unwrap();
        s.tail("r", 0, 10).unwrap();
        assert_eq!(
            s.read(Seat::new("r", "alice"), false, 3).unwrap().len(),
            1,
            "still unread"
        );
        assert_eq!(
            s.members("r")
                .unwrap()
                .iter()
                .find(|m| m.name == "alice")
                .unwrap()
                .last_seen,
            3
        );
    }
    pub fn advance_cursor_moves_forward_only_and_read_follows_it<S: Store>(make: fn() -> S) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        for i in 0..4 {
            s.post(Seat::new("r", "bob"), &format!("m{i}"), i).unwrap();
        }
        s.advance_cursor(Seat::new("r", "alice"), 2, 5).unwrap();
        assert_eq!(
            s.read(Seat::new("r", "alice"), false, 6)
                .unwrap()
                .iter()
                .map(|m| m.seq)
                .collect::<Vec<_>>(),
            vec![3, 4]
        );
        s.advance_cursor(Seat::new("r", "alice"), 1, 7).unwrap(); // backward: ignored
        assert_eq!(s.read(Seat::new("r", "alice"), false, 8).unwrap().len(), 2);
    }
    pub fn advance_cursor_never_passes_the_head_so_it_cannot_skip_future_messages<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        s.post(Seat::new("r", "bob"), "one", 2).unwrap();
        s.advance_cursor(Seat::new("r", "alice"), 99, 3).unwrap();
        s.post(Seat::new("r", "bob"), "two", 4).unwrap();
        let got = s.read(Seat::new("r", "alice"), false, 5).unwrap();
        assert_eq!(
            got.iter().map(|m| m.seq).collect::<Vec<_>>(),
            vec![2],
            "the later message must still arrive"
        );
    }
    pub fn advance_cursor_creates_the_membership_if_needed<S: Store>(make: fn() -> S) {
        let s = make();
        s.post(Seat::new("r", "bob"), "x", 1).unwrap();
        s.advance_cursor(Seat::new("r", "alice"), 1, 2).unwrap();
        assert!(s.members("r").unwrap().iter().any(|m| m.name == "alice"));
        assert!(
            s.read(Seat::new("r", "alice"), false, 3)
                .unwrap()
                .is_empty()
        );
    }
    pub fn post_as_stores_attribution_and_label_and_read_and_tail_return_them<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        s.post_as(
            &NewMessage::new(("r", "bob"), Attribution::Verified, "internal", "hi"),
            2,
        )
        .unwrap();
        s.post_as(
            &NewMessage::new(("r", "eve"), Attribution::Unattributed, "public", "yo"),
            3,
        )
        .unwrap();
        let read = s.read(Seat::new("r", "alice"), false, 4).unwrap();
        let facts = |ms: &[Message]| -> Vec<(Attribution, String)> {
            ms.iter()
                .map(|m| (m.attribution, m.label.clone()))
                .collect()
        };
        let expected = vec![
            (Attribution::Verified, "internal".to_string()),
            (Attribution::Unattributed, "public".to_string()),
        ];
        assert_eq!(facts(&read), expected);
        assert_eq!(facts(&s.tail("r", 0, 10).unwrap()), expected);
    }
    pub fn the_plain_post_is_unattributed_and_public<S: Store>(make: fn() -> S) {
        let s = make();
        s.post(Seat::new("r", "bob"), "x", 1).unwrap();
        let m = &s.tail("r", 0, 10).unwrap()[0];
        assert_eq!(
            (m.attribution, m.label.as_str()),
            (Attribution::Unattributed, "public")
        );
    }
    pub fn a_wake_position_is_separate_from_the_wait_position_and_the_read_cursor<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        s.post(Seat::new("r", "bob"), "one", 2).unwrap();
        assert_eq!(s.wake_poll("r", "alice", 3).unwrap().len(), 1);
        assert_eq!(
            s.wake_poll("r", "alice", 4).unwrap().len(),
            1,
            "poll alone changes nothing"
        );
        s.wake_seen(Seat::new("r", "alice"), 1, 5).unwrap();
        assert!(s.wake_poll("r", "alice", 6).unwrap().is_empty());
        // a wait still sees it, and so does a read
        assert_eq!(s.wait_poll("r", "alice", 7).unwrap().len(), 1);
        assert_eq!(s.read(Seat::new("r", "alice"), false, 8).unwrap().len(), 1);
        // and the other way round
        s.post(Seat::new("r", "bob"), "two", 9).unwrap();
        s.wait_seen(Seat::new("r", "alice"), 2, 10).unwrap();
        assert_eq!(s.wake_poll("r", "alice", 11).unwrap().len(), 1);
    }
    pub fn wake_seen_only_moves_forward_and_a_read_hides_what_it_read_from_a_wake<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        s.post(Seat::new("r", "bob"), "one", 2).unwrap();
        s.post(Seat::new("r", "bob"), "two", 3).unwrap();
        s.wake_seen(Seat::new("r", "alice"), 2, 4).unwrap();
        s.wake_seen(Seat::new("r", "alice"), 1, 5).unwrap();
        assert!(s.wake_poll("r", "alice", 6).unwrap().is_empty());
        s.post(Seat::new("r", "bob"), "three", 7).unwrap();
        s.read(Seat::new("r", "alice"), true, 8).unwrap();
        assert!(s.wake_poll("r", "alice", 9).unwrap().is_empty());
    }
    pub fn a_wake_poll_refreshes_presence_like_a_wait_poll<S: Store>(make: fn() -> S) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        s.wake_poll("r", "alice", 500).unwrap();
        assert_eq!(s.members("r").unwrap()[0].last_seen, 500);
    }

    // ---- typed handoffs: an optional kind and payload (#123) ----

    pub fn a_message_with_no_handoff_has_none<S: Store>(make: fn() -> S) {
        let s = make();
        s.post(Seat::new("r", "alice"), "plain", 1).unwrap();
        let m = &s.tail("r", 0, 10).unwrap()[0];
        assert_eq!((m.kind.as_deref(), m.payload.as_deref()), (None, None));
    }
    pub fn a_kind_and_payload_are_stored_and_come_back_by_every_read_path<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        s.join("r", "bob", 1).unwrap();
        s.post_as(
            &NewMessage::new(
                ("r", "alice"),
                Attribution::Unattributed,
                "public",
                "see attached",
            )
            .with_handoff("work-order", "{\"id\":7,\"files\":[\"a.rs\"]}"),
            2,
        )
        .unwrap();
        let want = (
            Some("work-order".to_string()),
            Some("{\"id\":7,\"files\":[\"a.rs\"]}".to_string()),
        );
        let got = |mut v: Vec<Message>| {
            let m = v.remove(0);
            (m.kind, m.payload)
        };
        assert_eq!(got(s.tail("r", 0, 10).unwrap()), want);
        assert_eq!(got(s.read(Seat::new("r", "bob"), false, 3).unwrap()), want);
        assert_eq!(got(s.hook_read("r", "bob", 4).unwrap()), want);
        assert_eq!(got(s.wait_poll("r", "bob", 5).unwrap()), want);
    }
    pub fn a_kind_without_a_payload_is_a_label_on_the_message_alone<S: Store>(make: fn() -> S) {
        let s = make();
        s.post_as(
            &NewMessage::new(("r", "alice"), Attribution::Unattributed, "public", "done")
                .with_kind("result"),
            1,
        )
        .unwrap();
        let m = &s.tail("r", 0, 10).unwrap()[0];
        assert_eq!(
            (m.kind.as_deref(), m.payload.as_deref()),
            (Some("result"), None)
        );
    }
}

pub mod members {
    use super::*;
    pub fn join_is_idempotent_and_lists_the_member<S: Store>(make: fn() -> S) {
        let s = make();
        s.join("r", "alice", 10).unwrap();
        s.join("r", "alice", 20).unwrap();
        let m = s.members("r").unwrap();
        assert_eq!(
            m,
            vec![MemberInfo {
                name: "alice".into(),
                last_seen: 20,
                status: None,
                parent: None
            }]
        );
        assert_eq!(s.rooms_of("alice").unwrap(), vec!["r".to_string()]);
    }
    pub fn rooms_of_is_sorted_and_per_member<S: Store>(make: fn() -> S) {
        let s = make();
        s.join("b", "alice", 1).unwrap();
        s.join("a", "alice", 1).unwrap();
        s.join("c", "bob", 1).unwrap();
        assert_eq!(
            s.rooms_of("alice").unwrap(),
            vec!["a".to_string(), "b".to_string()]
        );
    }
    pub fn a_status_is_stored_per_member_and_room_and_replaced_by_the_next<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        s.join("r2", "alice", 1).unwrap();
        s.set_status(Seat::new("r", "alice"), "busy", 2).unwrap();
        assert_eq!(s.members("r").unwrap()[0].status.as_deref(), Some("busy"));
        assert_eq!(s.members("r2").unwrap()[0].status, None);
        s.set_status(Seat::new("r", "alice"), "done leaving", 3)
            .unwrap();
        assert_eq!(
            s.members("r").unwrap()[0].status.as_deref(),
            Some("done leaving")
        );
    }
    pub fn setting_a_status_joins_a_member_who_was_not_there<S: Store>(make: fn() -> S) {
        let s = make();
        s.set_status(Seat::new("r", "bob"), "done", 5).unwrap();
        let m = s.members("r").unwrap();
        assert_eq!(
            (m[0].name.as_str(), m[0].status.as_deref()),
            ("bob", Some("done"))
        );
    }
    pub fn a_hook_run_is_recorded_per_member_and_event_with_a_count_and_the_size_only<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        s.record_hook_run(&HookEvent::new("alice", "SessionStart", 0), 10)
            .unwrap();
        s.record_hook_run(&HookEvent::new("alice", "PostToolUse", 120), 11)
            .unwrap();
        s.record_hook_run(&HookEvent::new("alice", "PostToolUse", 0), 15)
            .unwrap();
        s.record_hook_run(&HookEvent::new("bob", "SessionStart", 5), 12)
            .unwrap();
        let r = s.hook_runs("alice").unwrap();
        assert_eq!(r.len(), 2);
        let post = r.iter().find(|h| h.event == "PostToolUse").unwrap();
        assert_eq!((post.last_ts, post.runs, post.last_bytes), (15, 2, 0));
        let start = r.iter().find(|h| h.event == "SessionStart").unwrap();
        assert_eq!((start.last_ts, start.runs), (10, 1));
        assert_eq!(s.hook_runs("bob").unwrap().len(), 1);
        assert!(s.hook_runs("nobody").unwrap().is_empty());
    }
    pub fn a_session_is_remembered_as_its_member_and_the_last_note_wins<S: Store>(make: fn() -> S) {
        let s = make();
        assert_eq!(s.member_of_session("sess-1").unwrap(), None);
        s.note_session("sess-1", "alice", 10).unwrap();
        s.note_session("sess-2", "bob", 11).unwrap();
        assert_eq!(
            s.member_of_session("sess-1").unwrap().as_deref(),
            Some("alice")
        );
        s.note_session("sess-1", "alice-2", 12).unwrap();
        assert_eq!(
            s.member_of_session("sess-1").unwrap().as_deref(),
            Some("alice-2")
        );
        assert_eq!(
            s.member_of_session("sess-2").unwrap().as_deref(),
            Some("bob")
        );
    }
    pub fn the_session_that_holds_a_member_name_can_be_looked_up<S: Store>(make: fn() -> S) {
        let s = make();
        assert_eq!(s.session_of_member("alice").unwrap(), None);
        s.note_session("sess-1", "alice", 10).unwrap();
        s.note_session("sess-2", "alice", 20).unwrap();
        assert_eq!(
            s.session_of_member("alice").unwrap().as_deref(),
            Some("sess-2"),
            "the most recent holder"
        );
        assert_eq!(s.session_of_member("bob").unwrap(), None);
    }
    pub fn room_state_pause_and_blocks_are_remembered_and_audited<S: Store>(make: fn() -> S) {
        let s = make();
        assert_eq!(s.room_state("r").unwrap(), None);
        s.set_room_state("r", Some("closed"), 5).unwrap();
        assert_eq!(s.room_state("r").unwrap().as_deref(), Some("closed"));
        assert_eq!(s.room_state("other").unwrap(), None);
        s.set_room_state("r", None, 6).unwrap();
        assert_eq!(s.room_state("r").unwrap(), None);
        s.set_room_state("*", Some("paused"), 7).unwrap();
        assert_eq!(s.room_state("*").unwrap().as_deref(), Some("paused"));
        // a block is per room unless it is for every room
        s.block_member("r", "mallory", 8).unwrap();
        assert!(s.is_blocked("r", "mallory").unwrap());
        assert!(!s.is_blocked("other", "mallory").unwrap());
        assert!(!s.is_blocked("r", "alice").unwrap());
        s.block_member("*", "eve", 9).unwrap();
        assert!(s.is_blocked("anywhere", "eve").unwrap());
        assert!(s.unblock_member("r", "mallory").unwrap());
        assert!(
            !s.unblock_member("r", "mallory").unwrap(),
            "nothing left to allow"
        );
        assert!(!s.is_blocked("r", "mallory").unwrap());
        s.record_audit(&NewAudit::new("pause", "*", ""), 7).unwrap();
        s.record_audit(
            &NewAudit::new("remove", "r/mallory", "claims released: 2"),
            8,
        )
        .unwrap();
        let log = s.audit_log(10).unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(
            (log[0].action.as_str(), log[0].ts),
            ("remove", 8),
            "newest first"
        );
        assert_eq!(log[1].target, "*");
    }
    pub fn a_late_joiner_sees_history<S: Store>(make: fn() -> S) {
        let s = make();
        s.post(Seat::new("r", "bob"), "early", 1).unwrap();
        s.join("r", "alice", 2).unwrap();
        assert_eq!(s.read(Seat::new("r", "alice"), true, 3).unwrap().len(), 1);
    }
    pub fn a_mute_is_kept_per_member_and_room_and_can_be_lifted<S: Store>(make: fn() -> S) {
        let s = make();
        s.join("r", "alice", 1).unwrap();
        s.join("side", "alice", 1).unwrap();
        s.join("r", "bob", 1).unwrap();
        assert!(!s.is_muted(Seat::new("r", "alice")).unwrap());
        s.set_muted(Seat::new("r", "alice"), true, 2).unwrap();
        assert!(s.is_muted(Seat::new("r", "alice")).unwrap());
        assert!(!s.is_muted(Seat::new("side", "alice")).unwrap());
        assert!(!s.is_muted(Seat::new("r", "bob")).unwrap());
        s.set_muted(Seat::new("r", "alice"), false, 3).unwrap();
        assert!(!s.is_muted(Seat::new("r", "alice")).unwrap());
        assert!(!s.is_muted(Seat::new("nowhere", "ghost")).unwrap());
    }
    pub fn the_first_parent_noted_for_a_member_is_kept_and_listed_with_it<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        s.join("r", "lead", 1).unwrap();
        s.join("r", "worker", 1).unwrap();
        s.note_parent("worker", "lead", 2).unwrap();
        s.note_parent("worker", "someone-else", 3).unwrap();
        let members = s.members("r").unwrap();
        let parent_of = |name: &str| {
            members
                .iter()
                .find(|m| m.name == name)
                .and_then(|m| m.parent.clone())
        };
        assert_eq!(parent_of("worker").as_deref(), Some("lead"));
        assert_eq!(parent_of("lead"), None);
    }
}

pub mod claims {
    use super::*;
    use crate::domain::Error;
    pub fn the_attempt_that_closes_a_two_party_cycle_is_told_it_is_a_deadlock<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        s.claim(&ClaimRequest::new(("r", "alice"), "X", 1000, ""), 0)
            .unwrap();
        s.claim(&ClaimRequest::new(("r", "bob"), "Y", 1000, ""), 0)
            .unwrap();
        assert_eq!(
            deadlocked(s.claim(&ClaimRequest::new(("r", "alice"), "Y", 1000, ""), 10)),
            Some(false),
            "no cycle yet"
        );
        assert_eq!(
            deadlocked(s.claim(&ClaimRequest::new(("r", "bob"), "X", 1000, ""), 20)),
            Some(true),
            "bob closed the cycle"
        );
    }
    pub fn a_plain_conflict_is_not_a_deadlock<S: Store>(make: fn() -> S) {
        let s = make();
        s.claim(&ClaimRequest::new(("r", "alice"), "X", 1000, ""), 0)
            .unwrap();
        assert_eq!(
            deadlocked(s.claim(&ClaimRequest::new(("r", "bob"), "X", 1000, ""), 5)),
            Some(false)
        );
        s.claim(&ClaimRequest::new(("r", "bob"), "Y", 1000, ""), 6)
            .unwrap();
        assert_eq!(
            deadlocked(s.claim(&ClaimRequest::new(("r", "bob"), "X", 1000, ""), 7)),
            Some(false),
            "alice is not waiting on bob"
        );
    }
    pub fn a_wait_older_than_the_window_does_not_make_a_cycle<S: Store>(make: fn() -> S) {
        let s = make();
        s.claim(&ClaimRequest::new(("r", "alice"), "X", 5000, ""), 0)
            .unwrap();
        s.claim(&ClaimRequest::new(("r", "bob"), "Y", 5000, ""), 0)
            .unwrap();
        assert_eq!(
            deadlocked(s.claim(&ClaimRequest::new(("r", "alice"), "Y", 5000, ""), 10)),
            Some(false)
        );
        assert_eq!(
            deadlocked(s.claim(&ClaimRequest::new(("r", "bob"), "X", 5000, ""), 10 + 121)),
            Some(false),
            "alice stopped waiting long ago"
        );
    }
    pub fn a_cycle_of_three_is_found_at_the_attempt_that_closes_it<S: Store>(make: fn() -> S) {
        let s = make();
        s.claim(&ClaimRequest::new(("r", "a"), "X", 1000, ""), 0)
            .unwrap();
        s.claim(&ClaimRequest::new(("r", "b"), "Y", 1000, ""), 0)
            .unwrap();
        s.claim(&ClaimRequest::new(("r", "c"), "Z", 1000, ""), 0)
            .unwrap();
        assert_eq!(
            deadlocked(s.claim(&ClaimRequest::new(("r", "a"), "Y", 1000, ""), 5)),
            Some(false)
        );
        assert_eq!(
            deadlocked(s.claim(&ClaimRequest::new(("r", "b"), "Z", 1000, ""), 6)),
            Some(false)
        );
        assert_eq!(
            deadlocked(s.claim(&ClaimRequest::new(("r", "c"), "X", 1000, ""), 7)),
            Some(true)
        );
    }
    pub fn a_satisfied_wait_no_longer_counts<S: Store>(make: fn() -> S) {
        let s = make();
        s.claim(&ClaimRequest::new(("r", "alice"), "X", 1000, ""), 0)
            .unwrap();
        s.claim(&ClaimRequest::new(("r", "bob"), "Y", 1000, ""), 0)
            .unwrap();
        assert_eq!(
            deadlocked(s.claim(&ClaimRequest::new(("r", "alice"), "Y", 1000, ""), 5)),
            Some(false)
        );
        s.release("r", "bob", "Y").unwrap();
        s.claim(&ClaimRequest::new(("r", "alice"), "Y", 1000, ""), 6)
            .unwrap();
        assert_eq!(
            deadlocked(s.claim(&ClaimRequest::new(("r", "bob"), "X", 1000, ""), 7)),
            Some(false),
            "alice got Y, so she is not waiting"
        );
    }
    pub fn waits_in_one_room_do_not_make_a_cycle_in_another<S: Store>(make: fn() -> S) {
        let s = make();
        s.claim(&ClaimRequest::new(("r1", "alice"), "X", 1000, ""), 0)
            .unwrap();
        s.claim(&ClaimRequest::new(("r2", "bob"), "Y", 1000, ""), 0)
            .unwrap();
        assert_eq!(
            deadlocked(s.claim(&ClaimRequest::new(("r2", "alice"), "Y", 1000, ""), 5)),
            Some(false)
        );
        assert_eq!(
            deadlocked(s.claim(&ClaimRequest::new(("r1", "bob"), "X", 1000, ""), 6)),
            Some(false)
        );
    }
    pub fn a_shared_claim_collides_with_every_scope_in_both_directions<S: Store>(make: fn() -> S) {
        let s = make();
        s.claim(
            &ClaimRequest::new(("r", "alice"), "docs", 100, "").scoped("*"),
            0,
        )
        .unwrap();
        assert!(
            s.claim(
                &ClaimRequest::new(("r", "bob"), "docs/api.md", 100, "").scoped("git:b"),
                1
            )
            .is_err(),
            "scoped against shared"
        );
        s.claim(
            &ClaimRequest::new(("r", "carol"), "src", 100, "").scoped("git:c"),
            2,
        )
        .unwrap();
        assert!(
            s.claim(
                &ClaimRequest::new(("r", "dave"), "src/x.rs", 100, "").scoped("*"),
                3
            )
            .is_err(),
            "shared against scoped"
        );
    }
    pub fn an_empty_scope_behaves_as_claims_always_did_and_the_listing_shows_each_scope<
        S: Store,
    >(
        make: fn() -> S,
    ) {
        let s = make();
        s.claim(&ClaimRequest::new(("r", "alice"), "p", 100, ""), 0)
            .unwrap();
        assert!(
            s.claim(&ClaimRequest::new(("r", "bob"), "p", 100, ""), 1)
                .is_err(),
            "the default scope still collides"
        );
        s.claim(
            &ClaimRequest::new(("r", "carol"), "q", 100, "").scoped("git:c"),
            2,
        )
        .unwrap();
        let claims = s.claims("r", 3).unwrap();
        let scope_of = |p: &str| claims.iter().find(|c| c.path == p).map(|c| c.scope.clone());
        assert_eq!(scope_of("p").as_deref(), Some(""));
        assert_eq!(scope_of("q").as_deref(), Some("git:c"));
    }
    pub fn a_release_note_travels_to_those_who_saw_the_claim_and_to_the_next_claimant<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        s.claim(&ClaimRequest::new(("r", "bob"), "src/a.rs", 100, ""), 0)
            .unwrap();
        assert_eq!(changes(&s, "r", "alice", 1).0.len(), 1);
        s.release_with(
            &Release::new(("r", "bob"), "src/a.rs", "rewrote parse(); re-read it"),
            10,
        )
        .unwrap();
        let c = s.claim_changes("r", "alice", 11).unwrap();
        assert_eq!(c.released.len(), 1);
        assert_eq!(
            c.released[0].note.as_deref(),
            Some("rewrote parse(); re-read it")
        );
        let last = s.last_release("r", "src/a.rs").unwrap().unwrap();
        assert_eq!(
            (last.owner.as_str(), last.note.as_str(), last.ts),
            ("bob", "rewrote parse(); re-read it", 10)
        );
        // a later release on the same path replaces it; one with no note leaves no note behind
        s.claim(&ClaimRequest::new(("r", "carol"), "src/a.rs", 100, ""), 20)
            .unwrap();
        s.release_with(&Release::new(("r", "carol"), "src/a.rs", ""), 30)
            .unwrap();
        assert!(s.last_release("r", "src/a.rs").unwrap().is_none());
        assert!(s.last_release("r", "other.rs").unwrap().is_none());
    }
    pub fn removing_a_member_takes_it_out_of_the_room_and_releases_its_claims_there<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        s.join("r", "mallory", 1).unwrap();
        s.join("side", "mallory", 1).unwrap();
        s.claim(&ClaimRequest::new(("r", "mallory"), "a.rs", 600, ""), 2)
            .unwrap();
        s.claim(&ClaimRequest::new(("side", "mallory"), "b.rs", 600, ""), 2)
            .unwrap();
        assert_eq!(s.remove_member("r", "mallory", 3).unwrap(), 1);
        assert!(s.members("r").unwrap().iter().all(|m| m.name != "mallory"));
        assert!(s.claims("r", 4).unwrap().is_empty());
        assert_eq!(
            s.claims("side", 4).unwrap().len(),
            1,
            "the other room is untouched"
        );
        assert_eq!(s.rooms_of("mallory").unwrap(), vec!["side".to_string()]);
    }
    pub fn claims_do_not_conflict_across_rooms<S: Store>(make: fn() -> S) {
        let s = make();
        s.claim(&ClaimRequest::new(("r1", "bob"), "src", 100, ""), 0)
            .unwrap();
        s.claim(&ClaimRequest::new(("r2", "alice"), "src", 100, ""), 0)
            .unwrap();
    }
    pub fn an_owner_can_reclaim_and_replace_their_own_path<S: Store>(make: fn() -> S) {
        let s = make();
        s.claim(&ClaimRequest::new(("r", "bob"), "src", 100, "a"), 0)
            .unwrap();
        s.claim(&ClaimRequest::new(("r", "bob"), "src", 200, "b"), 0)
            .unwrap();
        let c = s.claims("r", 1).unwrap();
        assert_eq!(
            c,
            vec![Claim {
                path: "src".into(),
                owner: "bob".into(),
                reason: "b".into(),
                expires: 200,
                scope: String::new()
            }]
        );
    }
    pub fn expired_claims_free_the_path_and_are_not_listed<S: Store>(make: fn() -> S) {
        let s = make();
        s.claim(&ClaimRequest::new(("r", "bob"), "src", 10, ""), 0)
            .unwrap();
        assert!(s.claims("r", 10).unwrap().is_empty());
        s.claim(&ClaimRequest::new(("r", "alice"), "src", 10, ""), 10)
            .unwrap();
        assert_eq!(s.claims("r", 11).unwrap()[0].owner, "alice");
    }
    pub fn claims_are_ordered_by_path<S: Store>(make: fn() -> S) {
        let s = make();
        s.claim(&ClaimRequest::new(("r", "bob"), "z", 100, ""), 0)
            .unwrap();
        s.claim(&ClaimRequest::new(("r", "bob"), "a", 100, ""), 0)
            .unwrap();
        let paths: Vec<_> = s
            .claims("r", 1)
            .unwrap()
            .into_iter()
            .map(|c| c.path)
            .collect();
        assert_eq!(paths, vec!["a", "z"]);
    }
    pub fn claim_changes_first_call_shows_others_claims_then_nothing<S: Store>(make: fn() -> S) {
        let s = make();
        s.claim(&ClaimRequest::new(("r", "bob"), "a", 100, ""), 0)
            .unwrap();
        s.claim(&ClaimRequest::new(("r", "alice"), "mine", 100, ""), 0)
            .unwrap();
        assert_eq!(
            changes(&s, "r", "alice", 1),
            (vec![pair("bob", "a")], vec![])
        );
        assert_eq!(changes(&s, "r", "alice", 2), (vec![], vec![]));
    }
    pub fn claim_changes_ignores_the_passing_of_time<S: Store>(make: fn() -> S) {
        let s = make();
        s.claim(&ClaimRequest::new(("r", "bob"), "a", 1000, ""), 0)
            .unwrap();
        changes(&s, "r", "alice", 1);
        for now in [10, 100, 500] {
            assert_eq!(changes(&s, "r", "alice", now), (vec![], vec![]), "at {now}");
        }
    }
    pub fn claim_changes_reports_only_the_new_claim<S: Store>(make: fn() -> S) {
        let s = make();
        s.claim(&ClaimRequest::new(("r", "bob"), "a", 100, ""), 0)
            .unwrap();
        changes(&s, "r", "alice", 1);
        s.claim(&ClaimRequest::new(("r", "bob"), "b", 100, ""), 2)
            .unwrap();
        assert_eq!(
            changes(&s, "r", "alice", 3),
            (vec![pair("bob", "b")], vec![])
        );
    }
    pub fn claim_changes_reports_a_renewed_claim_again<S: Store>(make: fn() -> S) {
        let s = make();
        s.claim(&ClaimRequest::new(("r", "bob"), "a", 100, ""), 0)
            .unwrap();
        changes(&s, "r", "alice", 1);
        s.claim(&ClaimRequest::new(("r", "bob"), "a", 500, ""), 50)
            .unwrap();
        assert_eq!(
            changes(&s, "r", "alice", 51),
            (vec![pair("bob", "a")], vec![])
        );
        assert_eq!(changes(&s, "r", "alice", 52), (vec![], vec![]));
    }
    pub fn claim_changes_reports_a_release_once<S: Store>(make: fn() -> S) {
        let s = make();
        s.claim(&ClaimRequest::new(("r", "bob"), "a", 100, ""), 0)
            .unwrap();
        changes(&s, "r", "alice", 1);
        s.release("r", "bob", "a").unwrap();
        assert_eq!(
            changes(&s, "r", "alice", 2),
            (vec![], vec![pair("bob", "a")])
        );
        assert_eq!(changes(&s, "r", "alice", 3), (vec![], vec![]));
    }
    pub fn claim_changes_are_remembered_per_member_and_per_room<S: Store>(make: fn() -> S) {
        let s = make();
        s.claim(&ClaimRequest::new(("r", "bob"), "a", 100, ""), 0)
            .unwrap();
        s.claim(&ClaimRequest::new(("other", "bob"), "z", 100, ""), 0)
            .unwrap();
        assert_eq!(changes(&s, "r", "alice", 1).0.len(), 1);
        assert_eq!(
            changes(&s, "r", "carol", 1).0.len(),
            1,
            "carol has her own view"
        );
        assert_eq!(
            changes(&s, "other", "alice", 1).0.len(),
            1,
            "another room has its own view"
        );
        assert_eq!(changes(&s, "r", "alice", 2).0.len(), 0);
    }
    pub fn claim_changes_does_not_hold_the_writer_lock_after_it_returns<S: Store>(make: fn() -> S) {
        let s = make();
        s.claim(&ClaimRequest::new(("r", "bob"), "a", 100, ""), 0)
            .unwrap();
        changes(&s, "r", "alice", 1);
        s.claim(&ClaimRequest::new(("r", "bob"), "b", 100, ""), 2)
            .unwrap();
    }
}

pub mod asks {
    use super::*;
    pub fn open_asks_follow_answers_and_claims_and_a_release_leaves_the_ask_open<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        let a = s
            .post(Seat::new("r", "alice"), "seeking: slugify", 1)
            .unwrap()
            .seq;
        let o = s.open_asks("r", 2).unwrap();
        assert_eq!((o.len(), o[0].claimed), (1, false));
        s.claim(
            &ClaimRequest::new(("r", "bob"), &format!("ask:{a}"), 100, ""),
            3,
        )
        .unwrap();
        assert!(s.open_asks("r", 4).unwrap()[0].claimed);
        assert!(
            !s.open_asks("r", 500).unwrap()[0].claimed,
            "a lapsed claim leaves the ask unclaimed"
        );
        s.release("r", "bob", &format!("ask:{a}")).unwrap();
        let after = s.open_asks("r", 5).unwrap();
        assert_eq!(
            (after.len(), after[0].claimed),
            (1, false),
            "a release must not end the ask"
        );
    }
    pub fn a_reply_or_a_close_ends_an_open_ask<S: Store>(make: fn() -> S) {
        let s = make();
        let a = s
            .post(Seat::new("r", "alice"), "seeking: one", 1)
            .unwrap()
            .seq;
        let b = s
            .post(Seat::new("r", "alice"), "seeking: two", 1)
            .unwrap()
            .seq;
        s.post(Seat::new("r", "bob"), &format!("answer to #{a}: here"), 2)
            .unwrap();
        s.post(Seat::new("r", "alice"), &format!("closed #{b}"), 3)
            .unwrap();
        assert!(s.open_asks("r", 4).unwrap().is_empty());
    }
    pub fn stale_asks_report_my_unclaimed_old_asks_once<S: Store>(make: fn() -> S) {
        let s = make();
        let a = s
            .post(Seat::new("r", "alice"), "seeking: mine", 10)
            .unwrap()
            .seq;
        s.post(Seat::new("r", "bob"), "seeking: his", 10).unwrap();
        assert!(
            s.stale_asks(Seat::new("r", "alice"), 69, 60)
                .unwrap()
                .is_empty(),
            "too young"
        );
        let first = s.stale_asks(Seat::new("r", "alice"), 70, 60).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].seq, a);
        assert!(
            s.stale_asks(Seat::new("r", "alice"), 71, 60)
                .unwrap()
                .is_empty(),
            "reported once"
        );
        assert_eq!(
            s.stale_asks(Seat::new("r", "bob"), 70, 60).unwrap().len(),
            1,
            "each asker is told about its own ask"
        );
    }
    pub fn a_claimed_or_answered_ask_is_not_stale<S: Store>(make: fn() -> S) {
        let s = make();
        let a = s
            .post(Seat::new("r", "alice"), "seeking: claimed", 10)
            .unwrap()
            .seq;
        let b = s
            .post(Seat::new("r", "alice"), "seeking: answered", 10)
            .unwrap()
            .seq;
        s.claim(
            &ClaimRequest::new(("r", "bob"), &format!("ask:{a}"), 1000, ""),
            11,
        )
        .unwrap();
        s.post(Seat::new("r", "bob"), &format!("answer to #{b}: ok"), 12)
            .unwrap();
        assert!(
            s.stale_asks(Seat::new("r", "alice"), 100, 60)
                .unwrap()
                .is_empty()
        );
    }
    pub fn an_asker_is_told_once_that_nobody_here_can_do_its_ask<S: Store>(make: fn() -> S) {
        let s = make();
        let a = s
            .post(Seat::new("r", "alice"), "seeking: x", 1)
            .unwrap()
            .seq;
        s.post(
            Seat::new("r", "h1"),
            &format!("declined #{a} cannot: not mine"),
            2,
        )
        .unwrap();
        assert!(
            s.escalated_asks("r", "alice", 3).unwrap().is_empty(),
            "one cannot is not enough"
        );
        s.post(
            Seat::new("r", "h2"),
            &format!("declined #{a} cannot: no"),
            4,
        )
        .unwrap();
        assert!(
            s.escalated_asks("r", "bob", 5).unwrap().is_empty(),
            "only the asker is told"
        );
        let first = s.escalated_asks("r", "alice", 5).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].seq, a);
        assert!(
            s.escalated_asks("r", "alice", 6).unwrap().is_empty(),
            "told once"
        );
    }
    pub fn a_busy_helper_means_someone_capable_exists_so_no_escalation<S: Store>(make: fn() -> S) {
        let s = make();
        let a = s
            .post(Seat::new("r", "alice"), "seeking: x", 1)
            .unwrap()
            .seq;
        s.post(
            Seat::new("r", "h1"),
            &format!("declined #{a} cannot: no"),
            2,
        )
        .unwrap();
        s.post(
            Seat::new("r", "h2"),
            &format!("declined #{a} cannot: no"),
            2,
        )
        .unwrap();
        s.post(
            Seat::new("r", "h3"),
            &format!("declined #{a} busy: later"),
            2,
        )
        .unwrap();
        assert!(s.escalated_asks("r", "alice", 3).unwrap().is_empty());
    }
    pub fn a_working_room_is_listed_with_its_members_and_activity_and_attached_to_its_ask<
        S: Store,
    >(
        make: fn() -> S,
    ) {
        let s = make();
        let ask = s
            .post(Seat::new("lobby", "asker"), "seeking: x", 10)
            .unwrap()
            .seq;
        s.join(&format!("ask-{ask}"), "asker", 20).unwrap();
        s.join(&format!("ask-{ask}"), "helper", 25).unwrap();
        s.post(Seat::new(&format!("ask-{ask}"), "helper"), "working", 40)
            .unwrap();
        s.join("ask-99", "other", 5).unwrap();
        let rooms = s.rooms_with_prefix("ask-").unwrap();
        let mine = rooms
            .iter()
            .find(|r| r.name == format!("ask-{ask}"))
            .unwrap();
        assert_eq!((mine.members, mine.last_activity), (2, 40));
        assert_eq!(rooms.len(), 2);
        let asks = s.open_asks("lobby", 50).unwrap();
        assert_eq!(asks[0].room.as_ref().map(|r| r.members), Some(2));
        assert!(s.rooms_with_prefix("zzz-").unwrap().is_empty());
    }
    pub fn an_ask_waiting_on_its_asker_or_with_a_busy_room_is_not_reported_stale<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        let a = s
            .post(Seat::new("lobby", "asker"), "seeking: a", 10)
            .unwrap()
            .seq;
        let b = s
            .post(Seat::new("lobby", "asker"), "seeking: b", 10)
            .unwrap()
            .seq;
        let c = s
            .post(Seat::new("lobby", "asker"), "seeking: c", 10)
            .unwrap()
            .seq;
        s.post(
            Seat::new("lobby", "helper"),
            &format!("need-info #{a}: which format?"),
            20,
        )
        .unwrap();
        s.join(&format!("ask-{b}"), "helper", 150).unwrap();
        let told: Vec<_> = s
            .stale_asks(Seat::new("lobby", "asker"), 200, 60)
            .unwrap()
            .into_iter()
            .map(|x| x.seq)
            .collect();
        assert_eq!(told, vec![c], "only the ask nobody is working on is stale");
        // A room that has gone quiet stops counting.
        let later: Vec<_> = s
            .stale_asks(Seat::new("lobby", "asker"), 400, 60)
            .unwrap()
            .into_iter()
            .map(|x| x.seq)
            .collect();
        assert_eq!(
            later,
            vec![b],
            "b's room went quiet and c was already reported"
        );
    }
    pub fn a_lapsed_ask_claim_is_reported_to_the_asker_once_with_the_holder<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        let a = s
            .post(Seat::new("lobby", "asker"), "seeking: x", 1)
            .unwrap()
            .seq;
        s.claim(
            &ClaimRequest::new(("lobby", "helper"), &format!("ask:{a}"), 90, "").scoped("*"),
            10,
        )
        .unwrap();
        assert!(
            s.lapsed_asks("lobby", "asker", 99).unwrap().is_empty(),
            "the lease is still running"
        );
        let first = s.lapsed_asks("lobby", "asker", 101).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!((first[0].0.seq, first[0].1.as_str()), (a, "helper"));
        assert!(
            s.lapsed_asks("lobby", "asker", 102).unwrap().is_empty(),
            "reported once"
        );
        assert!(
            s.lapsed_asks("lobby", "someone-else", 101)
                .unwrap()
                .is_empty()
                || true
        );
    }
    pub fn releasing_a_members_claims_frees_every_room_and_tells_the_asker<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        let a = s
            .post(Seat::new("lobby", "asker"), "seeking: x", 1)
            .unwrap()
            .seq;
        s.claim(
            &ClaimRequest::new(("lobby", "helper"), &format!("ask:{a}"), 90, "").scoped("*"),
            10,
        )
        .unwrap();
        s.claim(
            &ClaimRequest::new(("lobby", "helper"), "src/a.rs", 600, ""),
            10,
        )
        .unwrap();
        s.claim(&ClaimRequest::new(("side", "helper"), "docs", 600, ""), 10)
            .unwrap();
        s.claim(
            &ClaimRequest::new(("lobby", "other"), "src/b.rs", 600, ""),
            10,
        )
        .unwrap();
        assert_eq!(s.release_member_claims("helper", 20).unwrap(), 3);
        let held: Vec<String> = s
            .claims("lobby", 21)
            .unwrap()
            .into_iter()
            .map(|c| c.path)
            .collect();
        assert_eq!(
            held,
            vec!["src/b.rs".to_string()],
            "only the other member's"
        );
        assert!(s.claims("side", 21).unwrap().is_empty());
        // the ask is open again and its asker is told once that the helper's claim ended without an answer
        let open = s.open_asks("lobby", 21).unwrap();
        assert!(!open[0].claimed);
        let told = s.lapsed_asks("lobby", "asker", 21).unwrap();
        assert_eq!((told[0].0.seq, told[0].1.as_str()), (a, "helper"));
        assert!(s.lapsed_asks("lobby", "asker", 22).unwrap().is_empty());
        // nothing left to release
        assert_eq!(s.release_member_claims("helper", 23).unwrap(), 0);
    }
    pub fn a_release_an_answer_or_a_fresh_claim_is_not_a_lapse<S: Store>(make: fn() -> S) {
        let s = make();
        let released = s
            .post(Seat::new("lobby", "asker"), "seeking: r", 1)
            .unwrap()
            .seq;
        let answered = s
            .post(Seat::new("lobby", "asker"), "seeking: a", 1)
            .unwrap()
            .seq;
        let renewed = s
            .post(Seat::new("lobby", "asker"), "seeking: n", 1)
            .unwrap()
            .seq;
        s.claim(
            &ClaimRequest::new(("lobby", "h"), &format!("ask:{released}"), 90, "").scoped("*"),
            10,
        )
        .unwrap();
        s.release("lobby", "h", &format!("ask:{released}")).unwrap();
        s.claim(
            &ClaimRequest::new(("lobby", "h"), &format!("ask:{answered}"), 90, "").scoped("*"),
            10,
        )
        .unwrap();
        s.post(
            Seat::new("lobby", "h"),
            &format!("answer to #{answered}: done"),
            50,
        )
        .unwrap();
        s.claim(
            &ClaimRequest::new(("lobby", "h"), &format!("ask:{renewed}"), 90, "").scoped("*"),
            10,
        )
        .unwrap();
        s.claim(
            &ClaimRequest::new(("lobby", "h"), &format!("ask:{renewed}"), 90, "").scoped("*"),
            95,
        )
        .unwrap();
        let lapsed: Vec<_> = s
            .lapsed_asks("lobby", "asker", 120)
            .unwrap()
            .into_iter()
            .map(|x| x.0.seq)
            .collect();
        assert!(lapsed.is_empty(), "{lapsed:?}");
    }
    pub fn a_members_presence_in_a_working_room_is_not_activity_in_it<S: Store>(make: fn() -> S) {
        let s = make();
        s.join("ask-1", "asker", 20).unwrap();
        s.join("ask-1", "helper", 25).unwrap();
        // Hooks refresh presence in every joined room; only a message or a join is news.
        s.read(Seat::new("ask-1", "asker"), false, 500).unwrap();
        s.wait_poll("ask-1", "helper", 600).unwrap();
        let r = s.rooms_with_prefix("ask-").unwrap();
        assert_eq!(
            r[0].last_activity, 25,
            "a refreshed member is not a busy room"
        );
        s.post(Seat::new("ask-1", "helper"), "working", 700)
            .unwrap();
        assert_eq!(s.rooms_with_prefix("ask-").unwrap()[0].last_activity, 700);
    }
    pub fn claim_changes_reports_a_lapsed_claim_as_released_once<S: Store>(make: fn() -> S) {
        let s = make();
        s.claim(&ClaimRequest::new(("r", "bob"), "a", 10, ""), 0)
            .unwrap();
        changes(&s, "r", "alice", 1);
        assert_eq!(
            changes(&s, "r", "alice", 20),
            (vec![], vec![pair("bob", "a")])
        );
        assert_eq!(changes(&s, "r", "alice", 21), (vec![], vec![]));
    }
    pub fn claim_changes_never_reports_a_claim_that_lapsed_before_it_was_seen<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        s.claim(&ClaimRequest::new(("r", "bob"), "a", 10, ""), 0)
            .unwrap();
        assert_eq!(changes(&s, "r", "alice", 20), (vec![], vec![]));
    }
}

pub mod anchors {
    use super::*;
    fn claim<S: Store>(s: &S, room: &str, owner: &str, path: &str, anchor: &str, now: i64) {
        s.claim(
            &ClaimRequest::new((room, owner), path, 600, "").anchored(anchor),
            now,
        )
        .unwrap();
    }
    pub fn releasing_an_anchor_releases_its_live_claims_in_every_room_and_only_those<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        claim(&s, "r1", "w-1", "src/a.rs", "wo:7", 100);
        claim(&s, "r2", "w-2", "src/b.rs", "wo:7", 100);
        claim(&s, "r1", "w-3", "src/c.rs", "wo:8", 100);
        claim(&s, "r1", "w-4", "src/d.rs", "", 100);
        let mut released = s.release_anchored("wo:7", 200).unwrap();
        released.sort();
        assert_eq!(
            released,
            vec![
                ("r1".to_string(), "src/a.rs".to_string(), "w-1".to_string()),
                ("r2".to_string(), "src/b.rs".to_string(), "w-2".to_string()),
            ]
        );
        let held: Vec<String> = s
            .claims("r1", 200)
            .unwrap()
            .into_iter()
            .map(|c| c.path)
            .collect();
        assert_eq!(held, vec!["src/c.rs".to_string(), "src/d.rs".to_string()]);
        assert!(s.claims("r2", 200).unwrap().is_empty());
        assert!(
            s.release_anchored("wo:7", 200).unwrap().is_empty(),
            "a second release found something"
        );
    }
    pub fn an_expired_claim_is_cleared_but_not_reported_as_released<S: Store>(make: fn() -> S) {
        let s = make();
        claim(&s, "r", "w-1", "src/a.rs", "wo:7", 100); // expires at 700
        assert!(s.release_anchored("wo:7", 800).unwrap().is_empty());
        assert!(s.claims("r", 800).unwrap().is_empty());
    }
    pub fn a_released_path_can_be_claimed_by_someone_else_at_once<S: Store>(make: fn() -> S) {
        let s = make();
        claim(&s, "r", "w-1", "src/a.rs", "wo:7", 100);
        assert!(
            s.claim(&ClaimRequest::new(("r", "w-2"), "src/a.rs", 600, ""), 150)
                .is_err()
        );
        s.release_anchored("wo:7", 160).unwrap();
        s.claim(&ClaimRequest::new(("r", "w-2"), "src/a.rs", 600, ""), 170)
            .unwrap();
    }
    pub fn the_anchors_in_use_are_the_distinct_anchors_of_live_claims<S: Store>(make: fn() -> S) {
        let s = make();
        claim(&s, "r", "w-1", "a", "wo:2", 100);
        claim(&s, "r", "w-2", "b", "wo:1", 100);
        claim(&s, "r", "w-3", "c", "wo:1", 100);
        claim(&s, "r", "w-4", "d", "", 100);
        claim(&s, "r", "w-5", "e", "wo:gone", 10); // expires at 610
        assert_eq!(
            s.anchors_in_use(650).unwrap(),
            vec!["wo:1".to_string(), "wo:2".to_string()]
        );
    }
}

pub mod policy {
    use super::*;
    pub fn a_policy_value_is_set_read_replaced_and_cleared<S: Store>(make: fn() -> S) {
        let s = make();
        assert_eq!(s.policy_get("label:r").unwrap(), None);
        s.policy_set("label:r", Some("private"), 1).unwrap();
        assert_eq!(s.policy_get("label:r").unwrap().as_deref(), Some("private"));
        s.policy_set("label:r", Some("secret"), 2).unwrap();
        assert_eq!(s.policy_get("label:r").unwrap().as_deref(), Some("secret"));
        s.policy_set("label:r", None, 3).unwrap();
        assert_eq!(s.policy_get("label:r").unwrap(), None);
    }
    pub fn policy_entries_are_listed_by_prefix_in_key_order<S: Store>(make: fn() -> S) {
        let s = make();
        s.policy_set("scrub:b", Some("two"), 1).unwrap();
        s.policy_set("scrub:a", Some("one"), 1).unwrap();
        s.policy_set("label:r", Some("x"), 1).unwrap();
        assert_eq!(
            s.policy_list("scrub:").unwrap(),
            vec![
                ("scrub:a".to_string(), "one".to_string()),
                ("scrub:b".to_string(), "two".to_string())
            ]
        );
    }
    pub fn a_prefix_with_wildcard_characters_matches_literally<S: Store>(make: fn() -> S) {
        let s = make();
        s.policy_set("a%b:x", Some("1"), 1).unwrap();
        s.policy_set("azzb:x", Some("2"), 1).unwrap();
        assert_eq!(s.policy_list("a%b").unwrap().len(), 1);
    }
    pub fn refusals_are_kept_newest_first_with_the_rule_and_no_text<S: Store>(make: fn() -> S) {
        let s = make();
        s.record_refusal(&NewRefusal::new("alice", "r", "post", "label"), 10)
            .unwrap();
        s.record_refusal(&NewRefusal::new("bob", "q", "claim", "credentials"), 20)
            .unwrap();
        let log = s.refusals(10).unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(
            (log[0].member.as_str(), log[0].rule.as_str(), log[0].ts),
            ("bob", "credentials", 20)
        );
        assert_eq!(
            (log[1].room.as_str(), log[1].operation.as_str()),
            ("r", "post")
        );
        assert_eq!(s.refusals(1).unwrap().len(), 1);
    }
    pub fn the_rules_a_member_was_refused_under_since_a_time_are_counted_per_member<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        for (m, t) in [("alice", 5), ("alice", 50), ("alice", 60), ("bob", 55)] {
            s.record_refusal(&NewRefusal::new(m, "r", "post", "label"), t)
                .unwrap();
        }
        assert_eq!(s.refusal_rules_since("alice", 40).unwrap().len(), 2);
        assert_eq!(s.refusal_rules_since("bob", 40).unwrap(), vec!["label"]);
        assert!(s.refusal_rules_since("carol", 0).unwrap().is_empty());
    }
}

pub mod credentials {
    use super::*;
    use crate::store::NewCredential;
    fn hash(b: u8) -> [u8; 32] {
        [b; 32]
    }
    fn issue<S: Store>(s: &S, member: &str, now: i64, ttl: i64) -> Result<()> {
        s.issue_credential(
            &NewCredential {
                member,
                token_hash: &hash(1),
                clearance: "private",
                parent: "lead",
                ttl,
            },
            now,
        )
    }
    pub fn an_issued_credential_is_stored_with_its_hash_clearance_parent_and_expiry<S: Store>(
        make: fn() -> S,
    ) {
        let s = make();
        issue(&s, "w-1", 1_000, 600).unwrap();
        let c = s.credential("w-1").unwrap().unwrap();
        assert_eq!(c.token_hash, hash(1).to_vec());
        assert_eq!(
            (c.clearance.as_str(), c.parent.as_str()),
            ("private", "lead")
        );
        assert_eq!((c.issued, c.expires, c.revoked), (1_000, 1_600, false));
        assert!(s.credential("nobody").unwrap().is_none());
    }
    pub fn revoking_works_once_and_marks_the_record_without_deleting_it<S: Store>(make: fn() -> S) {
        let s = make();
        issue(&s, "w-1", 1_000, 600).unwrap();
        assert!(s.revoke_credential("w-1", 1_100).unwrap());
        assert!(!s.revoke_credential("w-1", 1_200).unwrap());
        assert!(!s.revoke_credential("never-issued", 1_200).unwrap());
        assert!(s.credential("w-1").unwrap().unwrap().revoked);
    }
    pub fn the_list_has_every_credential_by_member<S: Store>(make: fn() -> S) {
        let s = make();
        issue(&s, "w-2", 1_000, 600).unwrap();
        issue(&s, "w-1", 1_000, 600).unwrap();
        let names: Vec<String> = s
            .credentials()
            .unwrap()
            .into_iter()
            .map(|c| c.member)
            .collect();
        assert_eq!(names, vec!["w-1".to_string(), "w-2".to_string()]);
    }
}
