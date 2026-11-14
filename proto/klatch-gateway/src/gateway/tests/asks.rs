use super::*;

#[test]
fn open_asks_go_through_the_gateway() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.post(&bob(), "r", "seeking: slugify").unwrap();
    let o = g.open_asks(&alice(), "r").unwrap();
    assert_eq!((o.len(), o[0].sender.as_str()), (1, "bob"));
}

#[test]
fn a_young_or_claimed_ask_is_not_stale() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.store
        .post(Seat::new("r", "alice"), "seeking: young", 90)
        .unwrap();
    let old = g
        .store
        .post(Seat::new("r", "alice"), "seeking: claimed", 10)
        .unwrap()
        .seq;
    g.claim(&bob(), "r", &format!("ask:{old}"), 500, "")
        .unwrap();
    assert!(g.digest(&alice()).unwrap().rooms[0].stale.is_empty());
}

#[test]
fn the_backlog_lists_others_open_unclaimed_asks_only_when_asked_for() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.store
        .post(Seat::new("r", "bob"), "seeking: open", 10)
        .unwrap();
    let claimed = g
        .store
        .post(Seat::new("r", "bob"), "seeking: taken", 10)
        .unwrap()
        .seq;
    g.store
        .post(Seat::new("r", "alice"), "seeking: mine", 10)
        .unwrap();
    g.claim(&bob(), "r", &format!("ask:{claimed}"), 500, "")
        .unwrap();
    assert!(
        g.digest(&alice())
            .unwrap()
            .rooms
            .iter()
            .all(|r| r.backlog.is_empty())
    );
    let d = g.digest_with(&alice(), true).unwrap();
    let b = &d.rooms[0].backlog;
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].text, "seeking: open");
}

#[test]
fn a_shared_claim_and_an_ask_claim_conflict_across_scopes() {
    let g = open();
    let (a, b) = (alice(), bob());
    g.set_scope(&a, "git:a").unwrap();
    g.set_scope(&b, "git:b").unwrap();
    g.claim_with(&a, "r", "docs", 100, "", true).unwrap();
    assert!(
        g.claim(&b, "r", "docs/api.md", 100, "").is_err(),
        "a scoped claim against a shared one"
    );
    g.claim(&a, "r", "ask:3", 100, "").unwrap();
    assert!(
        g.claim(&b, "r", "ask:3", 100, "").is_err(),
        "an ask is room-wide"
    );
}
