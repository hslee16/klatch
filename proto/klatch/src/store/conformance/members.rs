use super::*;
pub fn join_is_idempotent_and_lists_the_member<S: Store>(make: fn() -> S) {
    let s = make();
    s.join("r", "alice", 10).unwrap();
    s.join("r", "alice", 20).unwrap();
    // `joined` is optional (0 is unknown); the SQLite store reports it and its own test checks the value
    let m: Vec<MemberInfo> = s
        .members("r")
        .unwrap()
        .into_iter()
        .map(|m| MemberInfo { joined: 0, ..m })
        .collect();
    assert_eq!(
        m,
        vec![MemberInfo {
            status_ts: 0,
            name: "alice".into(),
            joined: 0,
            last_seen: 20,
            status: None,
            parent: None,
            parked: None,
            kind: None,
            place: None,
            mode: None,
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
pub fn a_status_is_stored_per_member_and_room_and_replaced_by_the_next<S: Store>(make: fn() -> S) {
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
pub fn the_first_parent_noted_for_a_member_is_kept_and_listed_with_it<S: Store>(make: fn() -> S) {
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
