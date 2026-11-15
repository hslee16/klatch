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
