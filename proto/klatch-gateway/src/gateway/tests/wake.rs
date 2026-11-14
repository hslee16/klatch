use super::*;

#[test]
fn unread_messages_from_others_count_and_your_own_do_not() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.post(&alice(), "r", "mine").unwrap();
    assert_eq!(g.wakeable(&alice(), "r").unwrap(), 0);
    g.post(&bob(), "r", "one").unwrap();
    g.post(&bob(), "r", "two").unwrap();
    assert_eq!(g.wakeable(&alice(), "r").unwrap(), 2);
}

#[test]
fn counting_moves_no_cursor_so_the_messages_stay_unread() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.post(&bob(), "r", "one").unwrap();
    g.wakeable(&alice(), "r").unwrap();
    assert_eq!(g.read(&alice(), "r", true).unwrap().len(), 1);
    assert_eq!(g.wakeable(&alice(), "r").unwrap(), 0);
}

#[test]
fn a_muted_room_counts_only_mentions() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.set_muted(&alice(), "r", true).unwrap();
    g.post(&bob(), "r", "chatter").unwrap();
    assert_eq!(g.wakeable(&alice(), "r").unwrap(), 0);
    g.post(&bob(), "r", "@alice look").unwrap();
    assert_eq!(g.wakeable(&alice(), "r").unwrap(), 1);
}

#[test]
fn counting_goes_through_the_guard_and_checks_the_name() {
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(1),
        Rc::new(SpyGuard::denying(&["locked"])),
    );
    assert!(is_refused(g.wakeable(&alice(), "locked")));
    assert!(is_invalid(g.wakeable(&alice(), "bad room")));
}

#[test]
fn counting_refreshes_presence_so_a_waiting_session_is_not_taken_for_gone() {
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    t.set(t.get() + 590);
    g.wakeable(&alice(), "r").unwrap();
    t.set(t.get() + 590);
    let who = g.who(&alice(), "r").unwrap();
    let me = who.iter().find(|m| m.name == "alice").unwrap();
    assert_eq!(
        t.get() - me.last_seen,
        590,
        "last seen should be the count's time"
    );
}
