use super::*;

#[test]
fn every_operation_validates_the_room_name_before_anything_else() {
    let g = open();
    for room in ["bad room", "a\nb", "", "x/y"] {
        let who = alice();
        assert!(
            matches!(g.join(&who, room), Err(Error::Invalid(_))),
            "join {room:?}"
        );
        assert!(
            matches!(g.post(&who, room, "x"), Err(Error::Invalid(_))),
            "post {room:?}"
        );
        assert!(
            matches!(g.read(&who, room, true), Err(Error::Invalid(_))),
            "read {room:?}"
        );
        assert!(
            matches!(g.who(&who, room), Err(Error::Invalid(_))),
            "who {room:?}"
        );
        assert!(
            matches!(g.claim(&who, room, "p", 10, ""), Err(Error::Invalid(_))),
            "claim {room:?}"
        );
        assert!(
            matches!(g.release(&who, room, "p"), Err(Error::Invalid(_))),
            "release {room:?}"
        );
        assert!(
            matches!(g.claims(&who, room), Err(Error::Invalid(_))),
            "claims {room:?}"
        );
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
    assert!(matches!(g.join(&bad, "r"), Err(Error::Invalid(_))));
    assert!(matches!(g.post(&bad, "r", "x"), Err(Error::Invalid(_))));
    assert!(matches!(g.digest(&bad), Err(Error::Invalid(_))));
    assert_eq!(g.store.calls.get(), 0);
}

#[test]
fn claim_paths_and_reasons_are_checked() {
    let g = open();
    let who = alice();
    for path in ["", "a\nb", "a\rb", "a\u{0}b", &"x".repeat(1025)] {
        assert!(
            matches!(g.claim(&who, "r", path, 10, ""), Err(Error::Invalid(_))),
            "path {path:?}"
        );
    }
    assert!(matches!(
        g.claim(&who, "r", "src", 10, "bad\nreason"),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        g.claim(&who, "r", "src", 10, &"y".repeat(501)),
        Err(Error::Invalid(_))
    ));
    g.claim(&who, "r", "src/a b.rs", 10, "a fine reason")
        .unwrap();
    // Only the valid claim reached the store: it looks up the member's scope, then claims.
    assert_eq!(g.store.calls.get(), 2);
}

#[test]
fn a_claim_needs_a_positive_ttl() {
    let g = open();
    assert!(matches!(
        g.claim(&alice(), "r", "src", 0, ""),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        g.claim(&alice(), "r", "src", -5, ""),
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
    g.claim(&who, "r", "src", 10, "").unwrap();
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
fn a_denied_operation_never_reaches_the_store_and_returns_the_guards_error() {
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(1),
        Rc::new(SpyGuard::denying(&["locked"])),
    );
    let who = alice();
    let err = g.post(&who, "locked", "x").unwrap_err();
    assert!(
        matches!(err, Error::Refused { ref rule, .. } if rule == "test.deny_room"),
        "{err}"
    );
    assert!(g.read(&who, "locked", true).is_err());
    assert!(g.claim(&who, "locked", "p", 10, "").is_err());
    assert_eq!(g.store.calls.get(), 0);
    g.post(&who, "open", "x").unwrap();
}

#[test]
fn store_failures_surface_as_errors_never_as_success() {
    let g = open();
    g.store.fail.set(true);
    let who = alice();
    assert!(matches!(g.post(&who, "r", "x"), Err(Error::Backend(_))));
    assert!(matches!(g.read(&who, "r", true), Err(Error::Backend(_))));
    assert!(matches!(
        g.claim(&who, "r", "p", 10, ""),
        Err(Error::Backend(_))
    ));
    assert!(matches!(g.claims(&who, "r"), Err(Error::Backend(_))));
    assert!(matches!(g.digest(&who), Err(Error::Backend(_))));
}

#[test]
fn tail_head_and_ack_validate_before_the_store_and_go_through_the_guard() {
    let guard = Rc::new(SpyGuard::allowing());
    let g = Gateway::new(SpyStore::new(), FixedClock(1), guard.clone());
    let who = alice();
    for room in ["bad room", "a\nb", ""] {
        assert!(matches!(g.tail(&who, room, 0, 10), Err(Error::Invalid(_))));
        assert!(matches!(g.head(&who, room), Err(Error::Invalid(_))));
        assert!(matches!(g.ack(&who, room, 1), Err(Error::Invalid(_))));
    }
    assert!(matches!(g.tail(&who, "r", -1, 10), Err(Error::Invalid(_))));
    assert!(matches!(g.tail(&who, "r", 0, 0), Err(Error::Invalid(_))));
    assert!(matches!(g.ack(&who, "r", -1), Err(Error::Invalid(_))));
    assert_eq!(g.store.calls.get(), 0);
    assert!(
        guard.seen.borrow().is_empty(),
        "invalid requests never reach the guard"
    );
    g.tail(&who, "r", 0, 10).unwrap();
    g.head(&who, "r").unwrap();
    g.ack(&who, "r", 0).unwrap();
    assert_eq!(
        *guard.seen.borrow(),
        vec![
            Operation::Subscribe { room: "r".into() },
            Operation::Subscribe { room: "r".into() },
            Operation::Ack { room: "r".into() },
        ]
    );
}

#[test]
fn a_denied_tail_never_reaches_the_store() {
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(1),
        Rc::new(SpyGuard::denying(&["locked"])),
    );
    assert!(matches!(
        g.tail(&alice(), "locked", 0, 10),
        Err(Error::Refused { .. })
    ));
    assert!(matches!(
        g.ack(&alice(), "locked", 1),
        Err(Error::Refused { .. })
    ));
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
fn a_bad_session_id_is_rejected_before_the_store() {
    let g = open();
    for bad in ["", "a b", "x\ny", &"s".repeat(81)] {
        assert!(
            matches!(g.note_session(&alice(), bad), Err(Error::Invalid(_))),
            "{bad:?}"
        );
    }
    assert_eq!(g.store.calls.get(), 0);
}

#[test]
fn a_bad_scope_is_rejected_before_the_store() {
    let g = open();
    for bad in ["", "has space", "*", "new\nline", &"s".repeat(81)] {
        assert!(
            matches!(g.set_scope(&alice(), bad), Err(Error::Invalid(_))),
            "{bad:?}"
        );
    }
    assert_eq!(g.store.calls.get(), 0);
}
