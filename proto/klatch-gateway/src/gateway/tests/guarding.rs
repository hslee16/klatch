use super::*;

#[test]
fn every_operation_validates_the_room_name_before_anything_else() {
    let g = open();
    let who = alice();
    for room in ["bad room", "a\nb", "", "x/y"] {
        let refused = [
            ("join", is_invalid(g.join(&who, room))),
            ("post", is_invalid(g.post(&who, room, "x"))),
            ("read", is_invalid(g.read(&who, room, true))),
            ("who", is_invalid(g.who(&who, room))),
            (
                "claim",
                is_invalid(g.claim(&who, room, &ClaimSpec::new("p", 10, ""))),
            ),
            ("release", is_invalid(g.release(&who, room, "p"))),
            ("claims", is_invalid(g.claims(&who, room))),
        ];
        all_true(&refused, &format!("{room:?}"));
    }
    assert_eq!(
        g.store.calls.get(),
        0,
        "an invalid request must not reach the store"
    );
}

#[test]
fn an_invalid_member_name_is_refused_everywhere() {
    let g = open();
    let bad = Principal::unattributed("bad name\nforged");
    let refused = [
        ("join", is_invalid(g.join(&bad, "r"))),
        ("post", is_invalid(g.post(&bad, "r", "x"))),
        ("digest", is_invalid(g.digest(&bad))),
    ];
    all_true(&refused, "by a bad member name");
    assert_eq!(g.store.calls.get(), 0);
}

#[test]
fn claim_paths_and_reasons_are_checked() {
    let g = open();
    let who = alice();
    for path in ["", "a\nb", "a\rb", "a\u{0}b", &"x".repeat(1025)] {
        assert!(
            matches!(
                g.claim(&who, "r", &ClaimSpec::new(path, 10, "")),
                Err(Error::Invalid(_))
            ),
            "path {path:?}"
        );
    }
    assert!(matches!(
        g.claim(&who, "r", &ClaimSpec::new("src", 10, "bad\nreason")),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        g.claim(&who, "r", &ClaimSpec::new("src", 10, &"y".repeat(501))),
        Err(Error::Invalid(_))
    ));
    g.claim(
        &who,
        "r",
        &ClaimSpec::new("src/a b.rs", 10, "a fine reason"),
    )
    .unwrap();
    // Only the valid claim reached the store: it looks up the member's scope, then claims.
    assert_eq!(g.store.calls.get(), 2);
}

#[test]
fn a_claim_needs_a_positive_ttl() {
    let g = open();
    assert!(matches!(
        g.claim(&alice(), "r", &ClaimSpec::new("src", 0, "")),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        g.claim(&alice(), "r", &ClaimSpec::new("src", -5, "")),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn the_guard_runs_once_per_operation_in_order_before_the_store() {
    let guard = Rc::new(SpyGuard::allowing());
    let g = Gateway::new(SpyStore::new(), FixedClock(1), guard.clone());
    let who = alice();
    g.join(&who, "r").unwrap();
    g.post(&who, "r", "hi").unwrap();
    g.read(&who, "r", false).unwrap();
    g.who(&who, "r").unwrap();
    g.claim(&who, "r", &ClaimSpec::new("src", 10, "")).unwrap();
    g.claims(&who, "r").unwrap();
    g.release(&who, "r", "src").unwrap();
    let kinds: Vec<&str> = guard
        .seen
        .borrow()
        .iter()
        .map(|o| match o {
            Operation::Join { .. } => "join",
            Operation::Post { .. } => "post",
            Operation::Read { .. } => "read",
            Operation::Who { .. } => "who",
            Operation::Claim { .. } => "claim",
            Operation::Claims { .. } => "claims",
            Operation::Release { .. } => "release",
            Operation::Subscribe { .. } => "subscribe",
            Operation::Status { .. } => "status",
            Operation::Ack { .. } => "ack",
            Operation::Admin { .. } => "admin",
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["join", "post", "read", "who", "claim", "claims", "release"]
    );
}

#[test]
fn the_guard_sees_the_post_body_so_a_label_check_can_judge_it() {
    let guard = Rc::new(SpyGuard::allowing());
    let g = Gateway::new(SpyStore::new(), FixedClock(1), guard.clone());
    g.post(&alice(), "r", "the body").unwrap();
    assert_eq!(
        guard.seen.borrow()[0],
        Operation::Post {
            room: "r".into(),
            body: "the body".into()
        }
    );
}

#[test]
fn a_denied_operation_returns_the_guards_error() {
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(1),
        Rc::new(SpyGuard::denying(&["locked"])),
    );
    let err = g.post(&alice(), "locked", "x").unwrap_err();
    assert!(
        matches!(err, Error::Refused { ref rule, .. } if rule == "test.deny_room"),
        "{err}"
    );
}

#[test]
fn a_denied_operation_never_reaches_the_store() {
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(1),
        Rc::new(SpyGuard::denying(&["locked"])),
    );
    let who = alice();
    let denied = [
        ("post", g.post(&who, "locked", "x").is_err()),
        ("read", g.read(&who, "locked", true).is_err()),
        (
            "claim",
            g.claim(&who, "locked", &ClaimSpec::new("p", 10, ""))
                .is_err(),
        ),
    ];
    all_true(&denied, "in a locked room");
    assert_eq!(g.store.calls.get(), 0);
    g.post(&who, "open", "x").unwrap();
}

#[test]
fn store_failures_surface_as_errors_never_as_success() {
    let g = open();
    g.store.fail.set(true);
    let who = alice();
    let failed = [
        ("post", is_backend(g.post(&who, "r", "x"))),
        ("read", is_backend(g.read(&who, "r", true))),
        (
            "claim",
            is_backend(g.claim(&who, "r", &ClaimSpec::new("p", 10, ""))),
        ),
        ("claims", is_backend(g.claims(&who, "r"))),
        ("digest", is_backend(g.digest(&who))),
    ];
    all_true(&failed, "must fail when the store does");
}

#[test]
fn tail_head_and_ack_validate_before_the_store_and_the_guard() {
    let guard = Rc::new(SpyGuard::allowing());
    let g = Gateway::new(SpyStore::new(), FixedClock(1), guard.clone());
    let who = alice();
    for room in ["bad room", "a\nb", ""] {
        let refused = [
            ("tail", is_invalid(g.tail(&who, room, (0, 10)))),
            ("head", is_invalid(g.head(&who, room))),
            ("ack", is_invalid(g.ack(&who, room, 1))),
        ];
        all_true(&refused, &format!("{room:?}"));
    }
    let bad_ranges = [
        ("negative start", is_invalid(g.tail(&who, "r", (-1, 10)))),
        ("empty limit", is_invalid(g.tail(&who, "r", (0, 0)))),
        ("negative ack", is_invalid(g.ack(&who, "r", -1))),
    ];
    all_true(&bad_ranges, "must be refused");
    assert_eq!(g.store.calls.get(), 0);
    assert!(
        guard.seen.borrow().is_empty(),
        "invalid requests never reach the guard"
    );
}

#[test]
fn tail_head_and_ack_go_through_the_guard() {
    let guard = Rc::new(SpyGuard::allowing());
    let g = Gateway::new(SpyStore::new(), FixedClock(1), guard.clone());
    let who = alice();
    g.tail(&who, "r", (0, 10)).unwrap();
    g.head(&who, "r").unwrap();
    g.ack(&who, "r", 0).unwrap();
    let seen = vec![
        Operation::Subscribe { room: "r".into() },
        Operation::Subscribe { room: "r".into() },
        Operation::Ack { room: "r".into() },
    ];
    assert_eq!(*guard.seen.borrow(), seen);
}

#[test]
fn a_denied_tail_never_reaches_the_store() {
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(1),
        Rc::new(SpyGuard::denying(&["locked"])),
    );
    let denied = [
        ("tail", is_refused(g.tail(&alice(), "locked", (0, 10)))),
        ("ack", is_refused(g.ack(&alice(), "locked", 1))),
    ];
    all_true(&denied, "in a locked room");
    assert_eq!(g.store.calls.get(), 0);
}

#[test]
fn an_empty_or_whitespace_only_body_is_refused_before_the_guard_and_the_store() {
    let guard = Rc::new(SpyGuard::allowing());
    let g = Gateway::new(SpyStore::new(), FixedClock(1), guard.clone());
    for body in ["", " ", "   \t ", "\n", " \r\n "] {
        assert!(
            matches!(g.post(&alice(), "r", body), Err(Error::Invalid(_))),
            "body {body:?}"
        );
    }
    assert_eq!(g.store.calls.get(), 0);
    assert!(guard.seen.borrow().is_empty());
    g.post(&alice(), "r", " x ").unwrap();
}

#[test]
fn a_board_is_refused_where_a_read_is_and_never_reaches_the_store() {
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(1),
        Rc::new(SpyGuard::denying(&["locked"])),
    );
    assert!(g.board(&alice(), "locked").is_err());
    assert_eq!(g.store.calls.get(), 0);
}

#[test]
fn a_status_in_a_refused_room_never_reaches_the_store() {
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(1),
        Rc::new(SpyGuard::denying(&["locked"])),
    );
    assert!(g.set_status(&alice(), "locked", "done").is_err());
    assert_eq!(g.store.calls.get(), 0);
}

#[test]
fn a_bad_status_is_rejected_before_the_store() {
    let g = open();
    for bad in ["", "   ", "a\nb", "x\u{7}y", &"z".repeat(81)] {
        let err = g.set_status(&alice(), "r", bad).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{bad:?}: {err}");
    }
    assert_eq!(g.store.calls.get(), 0);
}

#[test]
fn a_bad_event_or_member_name_is_rejected_before_the_store() {
    let g = open();
    for bad in ["", "a b", "x\ny", &"e".repeat(41)] {
        assert!(
            matches!(g.record_hook_run(&alice(), bad, 0), Err(Error::Invalid(_))),
            "{bad:?}"
        );
    }
    assert!(matches!(
        g.hook_runs(&alice(), "bad name"),
        Err(Error::Invalid(_))
    ));
    assert_eq!(g.store.calls.get(), 0);
}

#[test]
fn a_bad_session_id_or_scope_is_rejected_before_the_store() {
    let g = open();
    let sessions = ["", "a b", "x\ny", &"s".repeat(81)];
    let scopes = ["", "has space", "*", "new\nline", &"s".repeat(81)];
    let refused: Vec<(&str, bool)> = sessions
        .iter()
        .map(|bad| ("session id", is_invalid(g.note_session(&alice(), bad))))
        .chain(
            scopes
                .iter()
                .map(|bad| ("scope", is_invalid(g.set_scope(&alice(), bad)))),
        )
        .collect();
    all_true(&refused, "must be refused");
    assert_eq!(g.store.calls.get(), 0);
}
