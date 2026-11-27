use super::*;

#[test]
fn a_status_survives_reopening_the_database() {
    let path = temp_db("status-persist");
    {
        let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        s.join("r", "alice", 1).unwrap();
        s.set_status(Seat::new("r", "alice"), "busy", 2).unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(s.members("r").unwrap()[0].status.as_deref(), Some("busy"));
}
#[test]
fn hook_runs_survive_reopening_the_database() {
    let path = temp_db("hook-runs");
    {
        let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        s.record_hook_run(&HookEvent::new("alice", "SessionStart", 0), 10)
            .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(s.hook_runs("alice").unwrap()[0].runs, 1);
}
#[test]
fn remembered_sessions_survive_reopening_and_are_pruned_with_old_members() {
    let path = temp_db("sessions");
    {
        let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        s.note_session("old", "alice", 10).unwrap();
        s.note_session("new", "bob", 9_000).unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(
        s.member_of_session("old").unwrap().as_deref(),
        Some("alice")
    );
    s.prune_members(8_640, 9_001).unwrap();
    assert_eq!(
        s.member_of_session("old").unwrap(),
        None,
        "an old session is pruned"
    );
    assert_eq!(s.member_of_session("new").unwrap().as_deref(), Some("bob"));
}
#[test]
fn a_member_mode_is_kept_per_member_the_last_report_wins_and_pruning_takes_it() {
    let s = store();
    s.join("r", "alice", 950).unwrap();
    s.join("r", "bob", 1).unwrap();
    s.note_member_mode("alice", "bypassPermissions", 10)
        .unwrap();
    s.note_member_mode("bob", "default", 11).unwrap();
    s.note_member_mode("alice", "plan", 12).unwrap();
    assert_eq!(
        s.member_modes().unwrap(),
        vec![
            ("alice".to_string(), "plan".to_string(), 12),
            ("bob".to_string(), "default".to_string(), 11)
        ]
    );
    // bob has been gone past the grace period, alice has not
    s.prune_members(100, 1000).unwrap();
    let left: Vec<String> = s.member_modes().unwrap().into_iter().map(|m| m.0).collect();
    assert_eq!(left, vec!["alice".to_string()]);
}

#[test]
fn member_attributes_survive_reopening_keep_the_first_id_and_group_by_repository() {
    use crate::domain::MemberAttrs;
    let path = temp_db("member-attrs");
    {
        let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        assert_eq!(s.member_attrs("alice").unwrap(), None);
        s.note_member_attrs("alice", &MemberAttrs::new("m-1", "git:a-1", "/w/a"), 1)
            .unwrap();
        s.note_member_attrs("bob", &MemberAttrs::new("m-2", "git:a-1", "/w/a2"), 2)
            .unwrap();
        s.note_member_attrs("carol", &MemberAttrs::new("m-3", "git:b-2", "/w/b"), 3)
            .unwrap();
        // a later note changes the observation, never the id
        s.note_member_attrs("alice", &MemberAttrs::new("m-9", "git:b-2", "/w/b"), 4)
            .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(
        s.member_attrs("alice").unwrap(),
        Some(MemberAttrs::new("m-1", "git:b-2", "/w/b"))
    );
    assert_eq!(s.members_in_repository("git:a-1").unwrap(), vec!["bob"]);
    assert_eq!(
        s.members_in_repository("git:b-2").unwrap(),
        vec!["alice", "carol"]
    );
    assert!(s.members_in_repository("").unwrap().is_empty());
}

#[test]
fn a_status_remembers_when_it_was_set_and_a_return_clears_only_done() {
    let s = SqliteStore::open(&temp_db("status-ts"), &crate::store::insecure_test_key()).unwrap();
    let at = Seat::new("r", "alice");
    s.set_status(at, "busy", 7).unwrap();
    assert_eq!(s.members("r").unwrap()[0].status_ts, 7);
    s.clear_done(at, 8).unwrap();
    assert_eq!(s.members("r").unwrap()[0].status.as_deref(), Some("busy"));
    s.set_status(at, "done leaving", 9).unwrap();
    assert_eq!(s.members("r").unwrap()[0].status_ts, 9);
    s.clear_done(at, 10).unwrap();
    let m = &s.members("r").unwrap()[0];
    assert_eq!((m.status.as_deref(), m.status_ts), (None, 0));
}

// The feed room (#1404): gh-watch posts GitHub events there; a seat in it starts muted, so only an explicit read, a
// mention or an unmute shows the events to a member.
#[test]
fn a_seat_in_the_feed_room_starts_muted_however_it_is_made() {
    let s = SqliteStore::open(&temp_db("feed-muted"), &crate::store::insecure_test_key()).unwrap();
    let feed = crate::domain::FEED_ROOM;
    s.join(feed, "joined", 1).unwrap();
    s.post(Seat::new(feed, "poster"), "PR #1 opened", 2)
        .unwrap();
    s.read(Seat::new(feed, "reader"), true, 3).unwrap();
    s.advance_cursor(Seat::new(feed, "acker"), Seq(1), 4)
        .unwrap();
    for who in ["joined", "poster", "reader", "acker"] {
        assert!(s.is_muted(Seat::new(feed, who)).unwrap(), "{who}");
    }
}

#[test]
fn only_the_feed_room_starts_muted_and_a_member_can_unmute_it() {
    let s = SqliteStore::open(&temp_db("feed-only"), &crate::store::insecure_test_key()).unwrap();
    s.join("lobby", "alice", 1).unwrap();
    s.join("feed2", "alice", 1).unwrap();
    assert!(!s.is_muted(Seat::new("lobby", "alice")).unwrap());
    assert!(!s.is_muted(Seat::new("feed2", "alice")).unwrap());
    let at = Seat::new(crate::domain::FEED_ROOM, "alice");
    s.join(at.room, "alice", 1).unwrap();
    s.set_muted(at, false, 2).unwrap();
    s.join(at.room, "alice", 3).unwrap();
    s.read(at, true, 4).unwrap();
    assert!(!s.is_muted(at).unwrap(), "an unmute is kept");
}

#[test]
fn an_acknowledgement_makes_a_muted_feed_seat_but_never_mutes_an_existing_unmuted_one() {
    let s = SqliteStore::open(&temp_db("feed-ack"), &crate::store::insecure_test_key()).unwrap();
    let feed = crate::domain::FEED_ROOM;
    s.post(Seat::new(feed, "poster"), "PR #1 opened", 1)
        .unwrap();
    let kept = Seat::new(feed, "kept");
    s.join(feed, "kept", 2).unwrap();
    s.set_muted(kept, false, 3).unwrap();
    s.advance_cursor(kept, Seq(1), 4).unwrap();
    assert!(
        !s.is_muted(kept).unwrap(),
        "an acknowledgement keeps an unmute"
    );
    let fresh = Seat::new(feed, "fresh");
    s.advance_cursor(fresh, Seq(1), 5).unwrap();
    assert!(
        s.is_muted(fresh).unwrap(),
        "a seat made by an acknowledgement starts muted"
    );
}
