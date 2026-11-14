use super::*;

#[test]
fn a_digest_collects_unread_and_others_claims_and_advances_the_cursor() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.post(&bob(), "r", "ping").unwrap();
    g.claim(&bob(), "r", "src", 50, "").unwrap();
    g.claim(&alice(), "r", "docs", 50, "").unwrap();
    let d = g.digest(&alice()).unwrap();
    assert_eq!(d.now, 100);
    assert_eq!(d.rooms.len(), 1);
    assert_eq!(d.rooms[0].messages[0].body, "ping");
    assert_eq!(
        d.rooms[0]
            .claims
            .iter()
            .map(|c| c.owner.as_str())
            .collect::<Vec<_>>(),
        vec!["bob"]
    );
    assert!(g.digest(&alice()).unwrap().rooms[0].messages.is_empty());
}

#[test]
fn a_digest_tells_the_asker_once_that_its_ask_has_no_claim() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.store
        .post(Seat::new("r", "alice"), "seeking: a lib", 10)
        .unwrap();
    let d = g.digest(&alice()).unwrap();
    assert_eq!(d.rooms[0].stale.len(), 1);
    assert!(
        g.digest(&alice()).unwrap().rooms[0].stale.is_empty(),
        "reported once"
    );
}

#[test]
fn an_ask_claim_lease_is_capped_and_a_digest_tells_the_asker_once_when_it_lapses() {
    let g = Gateway::new(SpyStore::new(), FixedClock(1000), AllowAll);
    g.join(&alice(), "r").unwrap();
    g.store
        .post(Seat::new("r", "alice"), "seeking: a thing", 900)
        .unwrap();
    g.claim(&bob(), "r", "ask:1", 3600, "").unwrap();
    let held = g.claims(&alice(), "r").unwrap();
    assert_eq!(
        held[0].expires,
        1000 + 300,
        "an ask lease is at most five minutes"
    );
    let g2 = Gateway::new(SpyStore::new(), FixedClock(1000), AllowAll);
    g2.join(&alice(), "r").unwrap();
    g2.store
        .post(Seat::new("r", "alice"), "seeking: a thing", 900)
        .unwrap();
    g2.store
        .claim(
            &ClaimRequest::new(("r", "bob"), "ask:1", 90, "").scoped("*"),
            500,
        )
        .unwrap(); // lapsed at 590
    let told = g2.digest(&alice()).unwrap();
    assert_eq!(told.rooms[0].lapsed.len(), 1);
    assert_eq!(told.rooms[0].lapsed[0].1, "bob");
    assert!(
        g2.digest(&alice()).unwrap().rooms[0].lapsed.is_empty(),
        "told once"
    );
}

#[test]
fn a_digest_shows_a_claim_once_not_on_every_call() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.claim(&bob(), "r", "src", 50, "").unwrap();
    assert_eq!(g.digest(&alice()).unwrap().rooms[0].claims.len(), 1);
    for _ in 0..3 {
        let d = g.digest(&alice()).unwrap();
        assert!(d.rooms[0].claims.is_empty(), "unchanged claim repeated");
        assert!(d.rooms[0].released.is_empty());
    }
}

#[test]
fn a_digest_reports_a_release_once() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.claim(&bob(), "r", "src", 50, "").unwrap();
    g.digest(&alice()).unwrap();
    g.release(&bob(), "r", "src").unwrap();
    let d = g.digest(&alice()).unwrap();
    assert_eq!(d.rooms[0].released.len(), 1);
    assert_eq!(d.rooms[0].released[0].owner, "bob");
    assert_eq!(d.rooms[0].released[0].path, "src");
    assert!(g.digest(&alice()).unwrap().rooms[0].released.is_empty());
}

#[test]
fn a_digest_still_delivers_messages_when_claims_are_unchanged() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.claim(&bob(), "r", "src", 50, "").unwrap();
    g.digest(&alice()).unwrap();
    g.post(&bob(), "r", "later").unwrap();
    let d = g.digest(&alice()).unwrap();
    assert_eq!(d.rooms[0].messages.len(), 1);
    assert!(d.rooms[0].claims.is_empty());
}

#[test]
fn a_digest_reports_refused_rooms_and_still_returns_the_others() {
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(1),
        Rc::new(SpyGuard::denying(&["secretroom"])),
    );
    let who = alice();
    g.join(&who, "open").unwrap();
    g.store.inner_join_for_test("secretroom", "alice");
    g.post(&bob(), "open", "visible").unwrap();
    let d = g.digest(&who).unwrap();
    assert_eq!(d.rooms.len(), 1);
    assert_eq!(d.rooms[0].room, "open");
    assert_eq!(d.refused.len(), 1);
    assert_eq!(d.refused[0].room, "secretroom");
    assert!(d.refused[0].reason.contains("test.deny_room"));
}
