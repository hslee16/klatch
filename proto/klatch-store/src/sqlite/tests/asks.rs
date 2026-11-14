use super::*;

#[test]
fn open_asks_follow_answers_and_claims_and_a_release_leaves_the_ask_open() {
    let s = store();
    let a = s.post("r", "alice", "seeking: slugify", 1).unwrap().seq;
    let o = s.open_asks("r", 2).unwrap();
    assert_eq!((o.len(), o[0].claimed), (1, false));
    s.claim("r", "bob", &format!("ask:{a}"), 100, "", 3)
        .unwrap();
    assert!(s.open_asks("r", 4).unwrap()[0].claimed);
    assert!(
        !s.open_asks("r", 500).unwrap()[0].claimed,
        "a lapsed claim leaves the ask unclaimed"
    );
    s.release("r", "bob", &format!("ask:{a}")).unwrap();
    let after = s.open_asks("r", 5).unwrap();
    assert_eq!(
        (after.len(), after[0].claimed),
        (1, false),
        "a release must not end the ask"
    );
}

#[test]
fn a_reply_or_a_close_ends_an_open_ask() {
    let s = store();
    let a = s.post("r", "alice", "seeking: one", 1).unwrap().seq;
    let b = s.post("r", "alice", "seeking: two", 1).unwrap().seq;
    s.post("r", "bob", &format!("answer to #{a}: here"), 2)
        .unwrap();
    s.post("r", "alice", &format!("closed #{b}"), 3).unwrap();
    assert!(s.open_asks("r", 4).unwrap().is_empty());
}

#[test]
fn stale_asks_report_my_unclaimed_old_asks_once() {
    let s = store();
    let a = s.post("r", "alice", "seeking: mine", 10).unwrap().seq;
    s.post("r", "bob", "seeking: his", 10).unwrap();
    assert!(
        s.stale_asks("r", "alice", 69, 60).unwrap().is_empty(),
        "too young"
    );
    let first = s.stale_asks("r", "alice", 70, 60).unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].seq, a);
    assert!(
        s.stale_asks("r", "alice", 71, 60).unwrap().is_empty(),
        "reported once"
    );
    assert_eq!(
        s.stale_asks("r", "bob", 70, 60).unwrap().len(),
        1,
        "each asker is told about its own ask"
    );
}

#[test]
fn a_claimed_or_answered_ask_is_not_stale() {
    let s = store();
    let a = s.post("r", "alice", "seeking: claimed", 10).unwrap().seq;
    let b = s.post("r", "alice", "seeking: answered", 10).unwrap().seq;
    s.claim("r", "bob", &format!("ask:{a}"), 1000, "", 11)
        .unwrap();
    s.post("r", "bob", &format!("answer to #{b}: ok"), 12)
        .unwrap();
    assert!(s.stale_asks("r", "alice", 100, 60).unwrap().is_empty());
}

#[test]
fn an_asker_is_told_once_that_nobody_here_can_do_its_ask() {
    let s = store();
    let a = s.post("r", "alice", "seeking: x", 1).unwrap().seq;
    s.post("r", "h1", &format!("declined #{a} cannot: not mine"), 2)
        .unwrap();
    assert!(
        s.escalated_asks("r", "alice", 3).unwrap().is_empty(),
        "one cannot is not enough"
    );
    s.post("r", "h2", &format!("declined #{a} cannot: no"), 4)
        .unwrap();
    assert!(
        s.escalated_asks("r", "bob", 5).unwrap().is_empty(),
        "only the asker is told"
    );
    let first = s.escalated_asks("r", "alice", 5).unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].seq, a);
    assert!(
        s.escalated_asks("r", "alice", 6).unwrap().is_empty(),
        "told once"
    );
}

#[test]
fn a_busy_helper_means_someone_capable_exists_so_no_escalation() {
    let s = store();
    let a = s.post("r", "alice", "seeking: x", 1).unwrap().seq;
    s.post("r", "h1", &format!("declined #{a} cannot: no"), 2)
        .unwrap();
    s.post("r", "h2", &format!("declined #{a} cannot: no"), 2)
        .unwrap();
    s.post("r", "h3", &format!("declined #{a} busy: later"), 2)
        .unwrap();
    assert!(s.escalated_asks("r", "alice", 3).unwrap().is_empty());
}

#[test]
fn a_working_room_is_listed_with_its_members_and_activity_and_attached_to_its_ask() {
    let s = store();
    let ask = s.post("lobby", "asker", "seeking: x", 10).unwrap().seq;
    s.join(&format!("ask-{ask}"), "asker", 20).unwrap();
    s.join(&format!("ask-{ask}"), "helper", 25).unwrap();
    s.post(&format!("ask-{ask}"), "helper", "working", 40)
        .unwrap();
    s.join("ask-99", "other", 5).unwrap();
    let rooms = s.rooms_with_prefix("ask-").unwrap();
    let mine = rooms
        .iter()
        .find(|r| r.name == format!("ask-{ask}"))
        .unwrap();
    assert_eq!((mine.members, mine.last_activity), (2, 40));
    assert_eq!(rooms.len(), 2);
    let asks = s.open_asks("lobby", 50).unwrap();
    assert_eq!(asks[0].room.as_ref().map(|r| r.members), Some(2));
    assert!(s.rooms_with_prefix("zzz-").unwrap().is_empty());
}

#[test]
fn an_ask_waiting_on_its_asker_or_with_a_busy_room_is_not_reported_stale() {
    let s = store();
    let a = s.post("lobby", "asker", "seeking: a", 10).unwrap().seq;
    let b = s.post("lobby", "asker", "seeking: b", 10).unwrap().seq;
    let c = s.post("lobby", "asker", "seeking: c", 10).unwrap().seq;
    s.post(
        "lobby",
        "helper",
        &format!("need-info #{a}: which format?"),
        20,
    )
    .unwrap();
    s.join(&format!("ask-{b}"), "helper", 150).unwrap();
    let told: Vec<_> = s
        .stale_asks("lobby", "asker", 200, 60)
        .unwrap()
        .into_iter()
        .map(|x| x.seq)
        .collect();
    assert_eq!(told, vec![c], "only the ask nobody is working on is stale");
    // A room that has gone quiet stops counting.
    let later: Vec<_> = s
        .stale_asks("lobby", "asker", 400, 60)
        .unwrap()
        .into_iter()
        .map(|x| x.seq)
        .collect();
    assert_eq!(
        later,
        vec![b],
        "b's room went quiet and c was already reported"
    );
}

#[test]
fn a_lapsed_ask_claim_is_reported_to_the_asker_once_with_the_holder() {
    let s = store();
    let a = s.post("lobby", "asker", "seeking: x", 1).unwrap().seq;
    s.claim_in("lobby", "helper", "*", &format!("ask:{a}"), 90, "", 10)
        .unwrap();
    assert!(
        s.lapsed_asks("lobby", "asker", 99).unwrap().is_empty(),
        "the lease is still running"
    );
    let first = s.lapsed_asks("lobby", "asker", 101).unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!((first[0].0.seq, first[0].1.as_str()), (a, "helper"));
    assert!(
        s.lapsed_asks("lobby", "asker", 102).unwrap().is_empty(),
        "reported once"
    );
    assert!(
        s.lapsed_asks("lobby", "someone-else", 101)
            .unwrap()
            .is_empty()
            || true
    );
}

#[test]
fn releasing_a_members_claims_frees_every_room_and_tells_the_asker() {
    let s = store();
    let a = s.post("lobby", "asker", "seeking: x", 1).unwrap().seq;
    s.claim_in("lobby", "helper", "*", &format!("ask:{a}"), 90, "", 10)
        .unwrap();
    s.claim("lobby", "helper", "src/a.rs", 600, "", 10).unwrap();
    s.claim("side", "helper", "docs", 600, "", 10).unwrap();
    s.claim("lobby", "other", "src/b.rs", 600, "", 10).unwrap();
    assert_eq!(s.release_member_claims("helper", 20).unwrap(), 3);
    let held: Vec<String> = s
        .claims("lobby", 21)
        .unwrap()
        .into_iter()
        .map(|c| c.path)
        .collect();
    assert_eq!(
        held,
        vec!["src/b.rs".to_string()],
        "only the other member's"
    );
    assert!(s.claims("side", 21).unwrap().is_empty());
    // the ask is open again and its asker is told once that the helper's claim ended without an answer
    let open = s.open_asks("lobby", 21).unwrap();
    assert!(!open[0].claimed);
    let told = s.lapsed_asks("lobby", "asker", 21).unwrap();
    assert_eq!((told[0].0.seq, told[0].1.as_str()), (a, "helper"));
    assert!(s.lapsed_asks("lobby", "asker", 22).unwrap().is_empty());
    // nothing left to release
    assert_eq!(s.release_member_claims("helper", 23).unwrap(), 0);
}

#[test]
fn a_release_an_answer_or_a_fresh_claim_is_not_a_lapse() {
    let s = store();
    let released = s.post("lobby", "asker", "seeking: r", 1).unwrap().seq;
    let answered = s.post("lobby", "asker", "seeking: a", 1).unwrap().seq;
    let renewed = s.post("lobby", "asker", "seeking: n", 1).unwrap().seq;
    s.claim_in("lobby", "h", "*", &format!("ask:{released}"), 90, "", 10)
        .unwrap();
    s.release("lobby", "h", &format!("ask:{released}")).unwrap();
    s.claim_in("lobby", "h", "*", &format!("ask:{answered}"), 90, "", 10)
        .unwrap();
    s.post("lobby", "h", &format!("answer to #{answered}: done"), 50)
        .unwrap();
    s.claim_in("lobby", "h", "*", &format!("ask:{renewed}"), 90, "", 10)
        .unwrap();
    s.claim_in("lobby", "h", "*", &format!("ask:{renewed}"), 90, "", 95)
        .unwrap();
    let lapsed: Vec<_> = s
        .lapsed_asks("lobby", "asker", 120)
        .unwrap()
        .into_iter()
        .map(|x| x.0.seq)
        .collect();
    assert!(lapsed.is_empty(), "{lapsed:?}");
}

#[test]
fn a_members_presence_in_a_working_room_is_not_activity_in_it() {
    let s = store();
    s.join("ask-1", "asker", 20).unwrap();
    s.join("ask-1", "helper", 25).unwrap();
    // Hooks refresh presence in every joined room; only a message or a join is news.
    s.read("ask-1", "asker", false, 500).unwrap();
    s.wait_poll("ask-1", "helper", 600).unwrap();
    let r = s.rooms_with_prefix("ask-").unwrap();
    assert_eq!(
        r[0].last_activity, 25,
        "a refreshed member is not a busy room"
    );
    s.post("ask-1", "helper", "working", 700).unwrap();
    assert_eq!(s.rooms_with_prefix("ask-").unwrap()[0].last_activity, 700);
}

#[test]
fn claim_changes_reports_a_lapsed_claim_as_released_once() {
    let s = store();
    s.claim("r", "bob", "a", 10, "", 0).unwrap();
    changes(&s, "r", "alice", 1);
    assert_eq!(
        changes(&s, "r", "alice", 20),
        (vec![], vec![pair("bob", "a")])
    );
    assert_eq!(changes(&s, "r", "alice", 21), (vec![], vec![]));
}

#[test]
fn claim_changes_never_reports_a_claim_that_lapsed_before_it_was_seen() {
    let s = store();
    s.claim("r", "bob", "a", 10, "", 0).unwrap();
    assert_eq!(changes(&s, "r", "alice", 20), (vec![], vec![]));
}
