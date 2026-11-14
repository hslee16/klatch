use super::*;

#[test]
fn a_hook_run_is_recorded_for_the_caller_and_readable_by_anyone() {
    let g = open();
    g.record_hook_run(&alice(), "SessionStart", 0).unwrap();
    g.record_hook_run(&alice(), "SessionStart", 40).unwrap();
    let r = g.hook_runs(&bob(), "alice").unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(
        (
            r[0].event.as_str(),
            r[0].runs,
            r[0].last_bytes,
            r[0].last_ts
        ),
        ("SessionStart", 2, 40, 100)
    );
}

#[test]
fn pruning_goes_through_the_gateway_with_its_clock() {
    let g = Gateway::new(SpyStore::new(), FixedClock(10_000), AllowAll);
    g.store.join("r", "old", 1).unwrap();
    g.store.join("r", "fresh", 9_900).unwrap();
    assert_eq!(g.prune_members(&alice(), 5_000).unwrap(), 1);
    assert_eq!(g.who(&alice(), "r").unwrap().len(), 1);
}

#[test]
fn a_session_noted_by_a_member_is_remembered_as_that_member() {
    let g = open();
    g.note_session(&alice(), "sess-1").unwrap();
    assert_eq!(
        g.store.member_of_session("sess-1").unwrap().as_deref(),
        Some("alice")
    );
}
