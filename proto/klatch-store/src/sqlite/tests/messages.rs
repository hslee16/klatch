use super::*;

use klatch_core::Attribution;

#[test]
fn post_returns_increasing_ids_and_marks_sender_present() {
    let s = store();
    let a = s.post(Seat::new("r", "alice"), "one", 5).unwrap();
    let b = s.post(Seat::new("r", "alice"), "two", 6).unwrap();
    assert!(b.seq > a.seq);
    assert_eq!(s.members("r").unwrap()[0].last_seen, 6);
}

#[test]
fn read_excludes_own_messages_and_advances_the_cursor() {
    let s = store();
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

#[test]
fn a_hook_shows_a_message_once_but_read_still_returns_it() {
    let s = store();
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

#[test]
fn a_hook_skips_what_the_member_already_read() {
    let s = store();
    s.join("r", "alice", 1).unwrap();
    s.post(Seat::new("r", "bob"), "one", 2).unwrap();
    s.read(Seat::new("r", "alice"), true, 3).unwrap();
    assert!(s.hook_read("r", "alice", 4).unwrap().is_empty());
    s.post(Seat::new("r", "bob"), "two", 5).unwrap();
    let h = s.hook_read("r", "alice", 6).unwrap();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].body, "two");
}

#[test]
fn a_hook_shows_new_messages_after_an_earlier_hook_and_read_returns_all_unread() {
    let s = store();
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

#[test]
fn a_hook_excludes_own_messages() {
    let s = store();
    s.join("r", "alice", 1).unwrap();
    s.post(Seat::new("r", "alice"), "mine", 2).unwrap();
    assert!(s.hook_read("r", "alice", 3).unwrap().is_empty());
}

#[test]
fn wait_poll_changes_nothing_and_wait_seen_stops_the_next_wake_but_not_read() {
    let s = store();
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

#[test]
fn a_wait_poll_refreshes_the_waiters_presence_like_a_read() {
    let s = store();
    s.join("r", "alice", 1).unwrap();
    s.wait_poll("r", "alice", 9).unwrap();
    assert_eq!(s.members("r").unwrap()[0].last_seen, 9);
    s.read(Seat::new("r", "alice"), false, 12).unwrap();
    assert_eq!(s.members("r").unwrap()[0].last_seen, 12);
}

#[test]
fn wait_seen_only_moves_forward() {
    let s = store();
    s.join("r", "alice", 1).unwrap();
    s.post(Seat::new("r", "bob"), "one", 2).unwrap();
    s.post(Seat::new("r", "bob"), "two", 3).unwrap();
    s.wait_seen(Seat::new("r", "alice"), 2, 4).unwrap();
    s.wait_seen(Seat::new("r", "alice"), 1, 5).unwrap();
    assert!(s.wait_poll("r", "alice", 6).unwrap().is_empty());
}

#[test]
fn wait_poll_skips_what_was_read_and_own_messages() {
    let s = store();
    s.join("r", "alice", 1).unwrap();
    s.post(Seat::new("r", "bob"), "one", 2).unwrap();
    s.read(Seat::new("r", "alice"), true, 3).unwrap();
    s.post(Seat::new("r", "alice"), "mine", 4).unwrap();
    assert!(s.wait_poll("r", "alice", 5).unwrap().is_empty());
}

#[test]
fn pruning_removes_long_gone_members_and_their_bookkeeping_but_keeps_messages_and_recent_members() {
    let s = store();
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

#[test]
fn peek_does_not_advance_the_cursor() {
    let s = store();
    s.join("r", "alice", 1).unwrap();
    s.post(Seat::new("r", "bob"), "hi", 2).unwrap();
    assert_eq!(s.read(Seat::new("r", "alice"), false, 3).unwrap().len(), 1);
    assert_eq!(s.read(Seat::new("r", "alice"), false, 4).unwrap().len(), 1);
    assert_eq!(s.read(Seat::new("r", "alice"), true, 5).unwrap().len(), 1);
    assert!(s.read(Seat::new("r", "alice"), true, 6).unwrap().is_empty());
}

#[test]
fn rooms_are_isolated() {
    let s = store();
    s.post(Seat::new("r1", "bob"), "x", 1).unwrap();
    s.join("r2", "alice", 2).unwrap();
    assert!(
        s.read(Seat::new("r2", "alice"), true, 3)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn data_persists_across_connections_on_disk() {
    let dir = std::env::temp_dir().join(format!("klatch-store-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("rooms.db");
    {
        let s = SqliteStore::open(&path).unwrap();
        s.post(Seat::new("r", "bob"), "kept", 1).unwrap();
    }
    let s = SqliteStore::open(&path).unwrap();
    s.join("r", "alice", 2).unwrap();
    assert_eq!(
        s.read(Seat::new("r", "alice"), true, 3).unwrap()[0].body,
        "kept"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn seq_is_dense_per_room_starting_at_one() {
    let s = store();
    let a = s.post(Seat::new("r1", "bob"), "x", 1).unwrap();
    let b = s.post(Seat::new("r1", "bob"), "y", 2).unwrap();
    let c = s.post(Seat::new("r2", "bob"), "z", 3).unwrap();
    assert_eq!((a.seq, b.seq, c.seq), (1, 2, 1));
}

#[test]
fn message_ids_are_unique_and_well_formed() {
    let s = store();
    let mut seen = std::collections::HashSet::new();
    for i in 0..50 {
        let room = if i % 2 == 0 { "a" } else { "b" };
        let id = s.post(Seat::new(room, "bob"), "x", i).unwrap().id;
        assert_eq!(id.len(), 26, "id {id}");
        assert!(seen.insert(id));
    }
}

#[test]
fn read_returns_the_same_id_and_seq_that_post_returned() {
    let s = store();
    s.join("r", "alice", 1).unwrap();
    let p = s.post(Seat::new("r", "bob"), "hi", 2).unwrap();
    let m = s.read(Seat::new("r", "alice"), false, 3).unwrap();
    assert_eq!((m[0].id.as_str(), m[0].seq), (p.id.as_str(), p.seq));
}

#[test]
fn read_orders_by_seq_and_the_cursor_follows_seq() {
    let s = store();
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

#[test]
fn every_row_carries_tenant_local_and_a_member() {
    let s = store();
    s.post(Seat::new("r", "bob"), "x", 1).unwrap();
    s.claim(&ClaimRequest::new(("r", "bob"), "src", 10, ""), 1)
        .unwrap();
    let count = |sql: &str| -> i64 { s.conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(
        count("SELECT COUNT(*) FROM messages WHERE tenant='local' AND member=sender"),
        1
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM claims WHERE tenant='local' AND member=owner"),
        1
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM members WHERE tenant='local'"),
        1
    );
}

#[test]
fn the_cursor_advances_to_the_last_message_returned_never_past_it() {
    // A message that arrives between the select and the cursor update would be lost if the cursor
    // jumped to the room's current maximum. The cursor must stop at the last message returned.
    let s = store();
    s.join("r", "alice", 1).unwrap();
    s.post(Seat::new("r", "bob"), "one", 2).unwrap(); // seq 1
    s.post(Seat::new("r", "alice"), "mine", 3).unwrap(); // seq 2, her own
    let got = s.read(Seat::new("r", "alice"), true, 4).unwrap();
    assert_eq!(got.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![1]);
    let cursor: i64 = s
        .conn
        .query_row(
            "SELECT last_seq FROM members WHERE room='r' AND name='alice'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cursor, 1, "cursor must equal the last returned seq");
    s.post(Seat::new("r", "bob"), "two", 5).unwrap(); // seq 3
    assert_eq!(s.read(Seat::new("r", "alice"), true, 6).unwrap()[0].seq, 3);
}

#[test]
fn head_is_zero_for_an_empty_room_and_the_highest_seq_otherwise() {
    let s = store();
    assert_eq!(s.head("r").unwrap(), 0);
    s.post(Seat::new("r", "bob"), "a", 1).unwrap();
    s.post(Seat::new("r", "bob"), "b", 2).unwrap();
    s.post(Seat::new("other", "bob"), "c", 3).unwrap();
    assert_eq!(s.head("r").unwrap(), 2);
}

#[test]
fn tail_returns_messages_after_a_seq_in_order_from_every_sender() {
    let s = store();
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

#[test]
fn tail_honours_the_limit_and_the_room() {
    let s = store();
    for i in 0..5 {
        s.post(Seat::new("r", "bob"), &format!("m{i}"), i).unwrap();
    }
    s.post(Seat::new("other", "bob"), "x", 9).unwrap();
    let two = s.tail("r", 0, 2).unwrap();
    assert_eq!(two.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![1, 2]);
    assert!(s.tail("empty", 0, 10).unwrap().is_empty());
}

#[test]
fn tail_has_no_side_effects_on_cursors_or_presence() {
    let s = store();
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

#[test]
fn advance_cursor_moves_forward_only_and_read_follows_it() {
    let s = store();
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

#[test]
fn advance_cursor_never_passes_the_head_so_it_cannot_skip_future_messages() {
    let s = store();
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

#[test]
fn advance_cursor_creates_the_membership_if_needed() {
    let s = store();
    s.post(Seat::new("r", "bob"), "x", 1).unwrap();
    s.advance_cursor(Seat::new("r", "alice"), 1, 2).unwrap();
    assert!(s.members("r").unwrap().iter().any(|m| m.name == "alice"));
    assert!(
        s.read(Seat::new("r", "alice"), false, 3)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn post_as_stores_attribution_and_label_and_read_and_tail_return_them() {
    let s = store();
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

#[test]
fn the_plain_post_is_unattributed_and_public() {
    let s = store();
    s.post(Seat::new("r", "bob"), "x", 1).unwrap();
    let m = &s.tail("r", 0, 10).unwrap()[0];
    assert_eq!(
        (m.attribution, m.label.as_str()),
        (Attribution::Unattributed, "public")
    );
}

#[test]
fn a_wake_position_is_separate_from_the_wait_position_and_the_read_cursor() {
    let s = store();
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

#[test]
fn wake_seen_only_moves_forward_and_a_read_hides_what_it_read_from_a_wake() {
    let s = store();
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

#[test]
fn a_wake_poll_refreshes_presence_like_a_wait_poll() {
    let s = store();
    s.join("r", "alice", 1).unwrap();
    s.wake_poll("r", "alice", 500).unwrap();
    assert_eq!(s.members("r").unwrap()[0].last_seen, 500);
}
