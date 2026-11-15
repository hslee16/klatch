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
