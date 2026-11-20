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

#[test]
fn a_noted_parent_shows_in_who_and_the_first_one_is_kept() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    g.note_parent(&bob(), Some("alice")).unwrap();
    g.note_parent(&bob(), Some("carol")).unwrap();
    let parent_of_bob = g
        .who(&alice(), "r")
        .unwrap()
        .into_iter()
        .find(|m| m.name == "bob")
        .and_then(|m| m.parent);
    assert_eq!(parent_of_bob.as_deref(), Some("alice"));
}

#[test]
fn a_member_is_never_its_own_parent_and_a_bad_name_is_refused() {
    let g = open();
    assert!(is_invalid(g.note_parent(&alice(), Some("alice"))));
    assert!(is_invalid(
        g.note_parent(&alice(), Some("bad name\nforged"))
    ));
    assert_eq!(g.store.calls.get(), 0);
}

#[test]
fn a_member_with_no_parent_is_recorded_as_looked_up_and_shows_none() {
    let g = open();
    g.note_parent(&bob(), None).unwrap();
    g.join(&bob(), "r").unwrap();
    let who = g.who(&alice(), "r").unwrap();
    assert!(who.iter().all(|m| m.parent.is_none()));
}

/// #677: `who` carries the mode each member last reported, and none for a member whose hooks never ran.
#[test]
fn who_carries_the_last_reported_mode_and_none_for_a_member_that_never_reported() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    g.note_member_mode(&alice(), "default").unwrap();
    g.note_member_mode(&alice(), "bypassPermissions").unwrap();
    let who = g.who(&bob(), "r").unwrap();
    let mode = |n: &str| who.iter().find(|m| m.name == n).unwrap().mode.clone();
    assert_eq!(mode("alice").as_deref(), Some("bypassPermissions"));
    assert_eq!(mode("bob"), None);
}

/// #800: the mode check is the only thing between a hook's input and the store, so a string that is not a short word
/// is refused and never stored.
#[test]
fn a_permission_mode_that_is_not_a_short_word_is_refused_and_not_stored() {
    let g = open();
    let long = "m".repeat(41);
    for bad in [
        "",
        "bad mode",
        "plan-mode",
        "plan\n",
        "mode;drop",
        long.as_str(),
    ] {
        assert!(
            is_invalid(g.note_member_mode(&alice(), bad)),
            "{bad:?} was accepted"
        );
    }
    assert!(
        g.store.member_modes().unwrap().is_empty(),
        "a bad mode was stored"
    );
    let longest = "m".repeat(40);
    for good in ["plan", "acceptEdits", "bypassPermissions", longest.as_str()] {
        g.note_member_mode(&alice(), good).unwrap();
        assert_eq!(g.store.member_modes().unwrap()[0].1, good);
    }
}
