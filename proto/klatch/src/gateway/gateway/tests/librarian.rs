use super::*;

fn lib() -> Principal {
    Principal::unattributed("lib")
}

/// A room with one message in it and `lib` made the librarian.
fn with_librarian() -> (Gateway<SpyStore>, Rc<std::cell::Cell<i64>>) {
    // time moves between refusals so the breaker (five safety refusals in five minutes) stays out of it
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    g.post(&alice(), "r", "what happened").unwrap();
    g.admin_set_librarian(&admin(), "lib", true).unwrap();
    (g, t)
}

#[test]
fn a_librarian_can_look_but_not_touch() {
    let (g, _) = with_librarian();
    let msgs = g.read(&lib(), "r", false).unwrap();
    assert_eq!(msgs.len(), 1);
    assert!(g.who(&lib(), "r").is_ok());
    let history = g
        .history(&lib(), "r", crate::gateway::HistoryRange::default())
        .unwrap();
    assert_eq!(history.messages.len(), 1, "history is a query (#518)");
    assert!(g.claims(&lib(), "r").is_ok());
    assert!(g.board(&lib(), "r").is_ok());
    assert!(g.digest(&lib()).is_ok());
}

#[test]
fn a_librarian_cannot_post_claim_or_change_any_state() {
    let (g, t) = with_librarian();
    type Attempt<'a> = Box<dyn Fn() -> Option<Error> + 'a>;
    let attempts: Vec<(&str, Attempt)> = vec![
        ("join", Box::new(|| g.join(&lib(), "r").err())),
        ("post", Box::new(|| g.post(&lib(), "r", "hi").err())),
        (
            "claim",
            Box::new(|| g.claim(&lib(), "r", &ClaimSpec::new("p", 10, "")).err()),
        ),
        ("release", Box::new(|| g.release(&lib(), "r", "p").err())),
        (
            "status",
            Box::new(|| g.set_status(&lib(), "r", "done").err()),
        ),
        ("mute", Box::new(|| g.set_muted(&lib(), "r", true).err())),
        ("ack", Box::new(|| g.ack(&lib(), "r", 1).err())),
        (
            "advancing read",
            Box::new(|| g.read(&lib(), "r", true).err()),
        ),
        (
            "clear itself",
            Box::new(|| g.admin_set_clearance(&lib(), "lib", "private").err()),
        ),
        (
            "drop its own role",
            Box::new(|| g.admin_set_librarian(&lib(), "lib", false).err()),
        ),
        (
            "close a room",
            Box::new(|| g.admin_close_room(&lib(), "r", true).err()),
        ),
    ];
    for (name, attempt) in attempts {
        t.set(t.get() + 600);
        match attempt() {
            Some(Error::Refused { rule, .. }) => assert_eq!(rule, "read-only", "{name}"),
            other => panic!("{name} was not refused as read-only: {other:?}"),
        }
    }
    assert_eq!(
        g.read(&bob(), "r", false).unwrap().len(),
        1,
        "nothing was posted"
    );
    assert_eq!(g.clearance_of("lib").unwrap(), "public");
}

#[test]
fn a_librarian_is_held_to_the_label_checks() {
    let (g, _) = with_librarian();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    assert!(is_refused(g.read(&lib(), "vault", false)));
    g.admin_set_clearance(&admin(), "lib", "private").unwrap();
    assert!(g.read(&lib(), "vault", false).is_ok());
}

#[test]
fn a_refusal_of_the_librarian_is_recorded_without_the_text() {
    let (g, _) = with_librarian();
    let _ = g
        .post(&lib(), "r", "the-hostname-quokka.internal")
        .unwrap_err();
    let log = g.admin_refusals(&admin(), 10).unwrap();
    assert_eq!(log[0].rule, "read-only");
    assert_eq!(log[0].operation, "post");
    assert!(!format!("{log:?}").contains("quokka"));
}

#[test]
fn the_role_belongs_to_one_member_and_can_be_taken_away() {
    let (g, _) = with_librarian();
    assert!(g.post(&bob(), "r", "bob is not a librarian").is_ok());
    g.admin_set_librarian(&admin(), "lib", false).unwrap();
    assert!(g.post(&lib(), "r", "no longer a librarian").is_ok());
}

/// What `lib` looks like to the room: its `last_seen`, or `None` when it is not a member.
fn lib_seen(g: &Gateway<SpyStore>) -> Option<i64> {
    let members = g.who(&alice(), "r").unwrap();
    members
        .iter()
        .find(|m| m.name == "lib")
        .map(|m| m.last_seen)
}

/// Everything a librarian may do to look at a room (#499).
fn look_at_the_room(g: &Gateway<SpyStore>) {
    assert_eq!(g.read(&lib(), "r", false).unwrap().len(), 1);
    g.who(&lib(), "r").unwrap();
    g.claims(&lib(), "r").unwrap();
    g.board(&lib(), "r").unwrap();
    g.history(&lib(), "r", crate::gateway::HistoryRange::default())
        .unwrap();
    g.digest(&lib()).unwrap();
}

/// The waiting and watching paths a librarian may call (#750).
fn wait_and_watch(g: &Gateway<SpyStore>) {
    assert_eq!(g.wait_poll(&lib(), "r").unwrap().len(), 1);
    assert_eq!(g.wakeable(&lib(), "r").unwrap(), 1);
    assert_eq!(g.wake_summary(&lib(), "r").unwrap().0, 1);
    g.wake_mark(&lib(), "r").unwrap();
    g.wait_seen(&lib(), "r", 1).unwrap();
}

/// A member who joined, then was made the librarian, and then five seconds passed: the gateway and the
/// last-seen it had as a member.
fn a_former_member_made_librarian() -> (Gateway<SpyStore>, Option<i64>) {
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    g.post(&alice(), "r", "what happened").unwrap();
    g.join(&lib(), "r").unwrap();
    let before = lib_seen(&g);
    assert!(before.is_some());
    g.admin_set_librarian(&admin(), "lib", true).unwrap();
    t.set(t.get() + 5_000);
    (g, before)
}

#[test]
fn a_librarian_waiting_or_watching_does_not_join_the_room() {
    let (g, t) = with_librarian();
    t.set(t.get() + 5_000);
    wait_and_watch(&g);
    assert_eq!(lib_seen(&g), None, "waiting made the librarian a member");
}

#[test]
fn a_librarian_that_was_a_member_keeps_its_last_seen_when_it_waits_or_watches() {
    let (g, before) = a_former_member_made_librarian();
    wait_and_watch(&g);
    assert_eq!(lib_seen(&g), before, "waiting refreshed the presence");
}

#[test]
fn a_librarian_wait_and_wake_marks_write_no_position_so_the_next_poll_still_sees_it() {
    let (g, _) = with_librarian();
    g.wake_mark(&lib(), "r").unwrap();
    g.wait_seen(&lib(), "r", 1).unwrap();
    assert_eq!(g.wait_poll(&lib(), "r").unwrap().len(), 1);
    assert_eq!(g.wakeable(&lib(), "r").unwrap(), 1);
}

#[test]
fn an_ordinary_members_wait_and_watch_still_refresh_its_presence() {
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    g.post(&alice(), "r", "hi").unwrap();
    t.set(t.get() + 5_000);
    g.wait_poll(&bob(), "r").unwrap();
    g.wake_mark(&bob(), "r").unwrap();
    let seen = g
        .who(&alice(), "r")
        .unwrap()
        .iter()
        .find(|m| m.name == "bob")
        .map(|m| m.last_seen);
    assert_eq!(seen, Some(t.get()));
    assert_eq!(g.wakeable(&bob(), "r").unwrap(), 0, "the mark holds");
}

#[test]
fn a_librarian_looking_does_not_join_the_room() {
    let (g, t) = with_librarian();
    t.set(t.get() + 5_000);
    look_at_the_room(&g);
    assert_eq!(lib_seen(&g), None, "looking made the librarian a member");
}

#[test]
fn a_librarian_that_was_a_member_keeps_its_old_last_seen_when_it_looks() {
    let (g, before) = a_former_member_made_librarian();
    look_at_the_room(&g);
    assert_eq!(lib_seen(&g), before, "looking refreshed the presence");
}

#[test]
fn a_librarian_peek_still_returns_what_is_unread_and_moves_no_cursor() {
    let (g, _) = with_librarian();
    assert_eq!(g.read(&lib(), "r", false).unwrap().len(), 1);
    assert_eq!(
        g.read(&lib(), "r", false).unwrap().len(),
        1,
        "a peek moves no cursor"
    );
}

#[test]
fn an_ordinary_members_peek_still_refreshes_its_presence() {
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    t.set(t.get() + 5_000);
    g.read(&bob(), "r", false).unwrap();
    let seen = g
        .who(&alice(), "r")
        .unwrap()
        .iter()
        .find(|m| m.name == "bob")
        .map(|m| m.last_seen);
    assert_eq!(seen, Some(t.get()), "a member's peek is presence");
}

#[test]
fn granting_the_role_is_audited_and_checks_the_name() {
    let (g, _) = with_librarian();
    let audit = g.admin_audit(&admin(), 5).unwrap();
    assert_eq!(audit[0].action, "librarian");
    assert!(is_invalid(g.admin_set_librarian(
        &admin(),
        "bad name",
        true
    )));
}
