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
