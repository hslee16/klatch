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
    assert_eq!(s.hook_read(Seat::new("r", "alice"), 3).unwrap().len(), 1);
    assert!(
        s.hook_read(Seat::new("r", "alice"), 4).unwrap().is_empty(),
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
    assert!(s.hook_read(Seat::new("r", "alice"), 4).unwrap().is_empty());
    s.post(Seat::new("r", "bob"), "two", 5).unwrap();
    let h = s.hook_read(Seat::new("r", "alice"), 6).unwrap();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].body, "two");
}
pub fn a_hook_shows_new_messages_after_an_earlier_hook_and_read_returns_all_unread<S: Store>(
    make: fn() -> S,
) {
    let s = make();
    s.join("r", "alice", 1).unwrap();
    s.post(Seat::new("r", "bob"), "one", 2).unwrap();
    s.hook_read(Seat::new("r", "alice"), 3).unwrap();
    s.post(Seat::new("r", "bob"), "two", 4).unwrap();
    assert_eq!(
        s.hook_read(Seat::new("r", "alice"), 5).unwrap()[0].body,
        "two"
    );
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
    assert!(s.hook_read(Seat::new("r", "alice"), 3).unwrap().is_empty());
}
pub fn wait_poll_changes_nothing_and_wait_seen_stops_the_next_wake_but_not_read<S: Store>(
    make: fn() -> S,
) {
    let s = make();
    s.join("r", "alice", 1).unwrap();
    s.post(Seat::new("r", "bob"), "one", 2).unwrap();
    assert_eq!(s.wait_poll(Seat::new("r", "alice"), 3).unwrap().len(), 1);
    assert_eq!(
        s.wait_poll(Seat::new("r", "alice"), 4).unwrap().len(),
        1,
        "poll alone must not change state"
    );
    s.wait_seen(Seat::new("r", "alice"), Seq(1), 5).unwrap();
    assert!(s.wait_poll(Seat::new("r", "alice"), 6).unwrap().is_empty());
    assert_eq!(
        s.read(Seat::new("r", "alice"), false, 7).unwrap().len(),
        1,
        "seen by a wait is still unread"
    );
    s.post(Seat::new("r", "bob"), "two", 8).unwrap();
    let w = s.wait_poll(Seat::new("r", "alice"), 9).unwrap();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].body, "two");
}
pub fn a_wait_poll_refreshes_the_waiters_presence_like_a_read<S: Store>(make: fn() -> S) {
    let s = make();
    s.join("r", "alice", 1).unwrap();
    s.wait_poll(Seat::new("r", "alice"), 9).unwrap();
    assert_eq!(s.members("r").unwrap()[0].last_seen, 9);
    s.read(Seat::new("r", "alice"), false, 12).unwrap();
    assert_eq!(s.members("r").unwrap()[0].last_seen, 12);
}
pub fn wait_seen_only_moves_forward<S: Store>(make: fn() -> S) {
    let s = make();
    s.join("r", "alice", 1).unwrap();
    s.post(Seat::new("r", "bob"), "one", 2).unwrap();
    s.post(Seat::new("r", "bob"), "two", 3).unwrap();
    s.wait_seen(Seat::new("r", "alice"), Seq(2), 4).unwrap();
    s.wait_seen(Seat::new("r", "alice"), Seq(1), 5).unwrap();
    assert!(s.wait_poll(Seat::new("r", "alice"), 6).unwrap().is_empty());
}
pub fn wait_poll_skips_what_was_read_and_own_messages<S: Store>(make: fn() -> S) {
    let s = make();
    s.join("r", "alice", 1).unwrap();
    s.post(Seat::new("r", "bob"), "one", 2).unwrap();
    s.read(Seat::new("r", "alice"), true, 3).unwrap();
    s.post(Seat::new("r", "alice"), "mine", 4).unwrap();
    assert!(s.wait_poll(Seat::new("r", "alice"), 5).unwrap().is_empty());
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
pub fn words_returns_only_the_messages_with_a_prefix_however_deep_they_are<S: Store>(
    make: fn() -> S,
) {
    let s = make();
    s.post(Seat::new("r", "alice"), "working: old task", 1)
        .unwrap();
    for i in 0..5100 {
        s.post(Seat::new("r", "bob"), &format!("chatter {i}"), 2)
            .unwrap();
    }
    s.post(Seat::new("r", "alice"), "checkpoint: halfway", 3)
        .unwrap();
    s.post(Seat::new("r", "bob"), "Working: wrong case", 4)
        .unwrap();
    s.post(Seat::new("r", "bob"), "x working: not a prefix", 5)
        .unwrap();
    s.post(Seat::new("r", "carol"), "%: a wildcard is not a prefix", 6)
        .unwrap();
    s.post(Seat::new("other", "alice"), "working: other room", 7)
        .unwrap();
    let got = s.words("r", &["working:", "checkpoint"]).unwrap();
    let bodies: Vec<&str> = got.iter().map(|m| m.body.as_str()).collect();
    assert_eq!(bodies, vec!["working: old task", "checkpoint: halfway"]);
    assert!(got[0].seq < got[1].seq, "oldest first");
    assert_eq!(got[0].sender, "alice");
    assert_eq!(got[0].ts, 1);
    assert!(s.words("r", &[]).unwrap().is_empty());
    assert!(s.words("empty", &["working:"]).unwrap().is_empty());
    assert_eq!(s.words("r", &["%"]).unwrap().len(), 1, "literal, not LIKE");
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
    s.advance_cursor(Seat::new("r", "alice"), Seq(2), 5)
        .unwrap();
    assert_eq!(
        s.read(Seat::new("r", "alice"), false, 6)
            .unwrap()
            .iter()
            .map(|m| m.seq)
            .collect::<Vec<_>>(),
        vec![3, 4]
    );
    s.advance_cursor(Seat::new("r", "alice"), Seq(1), 7)
        .unwrap(); // backward: ignored
    assert_eq!(s.read(Seat::new("r", "alice"), false, 8).unwrap().len(), 2);
}
pub fn advance_cursor_never_passes_the_head_so_it_cannot_skip_future_messages<S: Store>(
    make: fn() -> S,
) {
    let s = make();
    s.join("r", "alice", 1).unwrap();
    s.post(Seat::new("r", "bob"), "one", 2).unwrap();
    s.advance_cursor(Seat::new("r", "alice"), Seq(99), 3)
        .unwrap();
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
    s.advance_cursor(Seat::new("r", "alice"), Seq(1), 2)
        .unwrap();
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
    assert_eq!(s.wake_poll(Seat::new("r", "alice"), 3).unwrap().len(), 1);
    assert_eq!(
        s.wake_poll(Seat::new("r", "alice"), 4).unwrap().len(),
        1,
        "poll alone changes nothing"
    );
    s.wake_seen(Seat::new("r", "alice"), Seq(1), 5).unwrap();
    assert!(s.wake_poll(Seat::new("r", "alice"), 6).unwrap().is_empty());
    // a wait still sees it, and so does a read
    assert_eq!(s.wait_poll(Seat::new("r", "alice"), 7).unwrap().len(), 1);
    assert_eq!(s.read(Seat::new("r", "alice"), false, 8).unwrap().len(), 1);
    // and the other way round
    s.post(Seat::new("r", "bob"), "two", 9).unwrap();
    s.wait_seen(Seat::new("r", "alice"), Seq(2), 10).unwrap();
    assert_eq!(s.wake_poll(Seat::new("r", "alice"), 11).unwrap().len(), 1);
}
pub fn wake_seen_only_moves_forward_and_a_read_hides_what_it_read_from_a_wake<S: Store>(
    make: fn() -> S,
) {
    let s = make();
    s.join("r", "alice", 1).unwrap();
    s.post(Seat::new("r", "bob"), "one", 2).unwrap();
    s.post(Seat::new("r", "bob"), "two", 3).unwrap();
    s.wake_seen(Seat::new("r", "alice"), Seq(2), 4).unwrap();
    s.wake_seen(Seat::new("r", "alice"), Seq(1), 5).unwrap();
    assert!(s.wake_poll(Seat::new("r", "alice"), 6).unwrap().is_empty());
    s.post(Seat::new("r", "bob"), "three", 7).unwrap();
    s.read(Seat::new("r", "alice"), true, 8).unwrap();
    assert!(s.wake_poll(Seat::new("r", "alice"), 9).unwrap().is_empty());
}
pub fn a_wake_poll_refreshes_presence_like_a_wait_poll<S: Store>(make: fn() -> S) {
    let s = make();
    s.join("r", "alice", 1).unwrap();
    s.wake_poll(Seat::new("r", "alice"), 500).unwrap();
    assert_eq!(s.members("r").unwrap()[0].last_seen, 500);
}

// ---- typed handoffs: an optional kind and payload (#123) ----

pub fn a_message_with_no_handoff_has_none<S: Store>(make: fn() -> S) {
    let s = make();
    s.post(Seat::new("r", "alice"), "plain", 1).unwrap();
    let m = &s.tail("r", 0, 10).unwrap()[0];
    assert_eq!((m.kind.as_deref(), m.payload.as_deref()), (None, None));
}
pub fn a_kind_and_payload_are_stored_and_come_back_by_every_read_path<S: Store>(make: fn() -> S) {
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
    assert_eq!(got(s.hook_read(Seat::new("r", "bob"), 4).unwrap()), want);
    assert_eq!(got(s.wait_poll(Seat::new("r", "bob"), 5).unwrap()), want);
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
