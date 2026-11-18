//! A member lists its own recorded refusals (#678): rule, room, operation and time, never the text, and never another
//! member's.

use super::*;

const TOKEN: &str = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";

/// A gateway where `vault` is private; alice, bob and carol hold no clearance. The clock advances between calls so the
/// breaker (five safety refusals in five minutes) stays out of it.
fn vault() -> (Gateway<SpyStore>, Rc<std::cell::Cell<i64>>) {
    let (g, t) = timed();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    (g, t)
}

fn tick(t: &Rc<std::cell::Cell<i64>>) {
    t.set(t.get() + 10);
}

#[test]
fn a_member_with_no_refusal_lists_nothing() {
    let (g, _) = vault();
    assert!(g.my_refusals(&alice(), 10).unwrap().is_empty());
}

#[test]
fn a_member_lists_its_refusals_newest_first_with_rule_room_operation_and_time() {
    let (g, t) = vault();
    let first = t.get();
    let _ = g.post(&alice(), "vault", "first try").unwrap_err();
    tick(&t);
    let second = t.get();
    let _ = g
        .post(&alice(), "town", &format!("key {TOKEN}"))
        .unwrap_err();
    let mine = g.my_refusals(&alice(), 10).unwrap();
    assert_eq!(mine.len(), 2);
    let view: Vec<_> = mine
        .iter()
        .map(|r| {
            (
                r.member.as_str(),
                r.room.as_str(),
                r.operation.as_str(),
                r.rule.as_str(),
                r.ts,
            )
        })
        .collect();
    assert_eq!(
        view,
        vec![
            ("alice", "town", "post", "credentials", second),
            ("alice", "vault", "post", "clearance", first),
        ]
    );
}

#[test]
fn a_member_never_sees_another_members_refusal() {
    let (g, t) = vault();
    let _ = g.post(&bob(), "vault", "bob was here").unwrap_err();
    tick(&t);
    let _ = g.post(&alice(), "vault", "alice was here").unwrap_err();
    tick(&t);
    let _ = g.post(&bob(), "vault", "bob again").unwrap_err();
    let mine = g.my_refusals(&alice(), 10).unwrap();
    assert_eq!(mine.len(), 1);
    assert!(mine.iter().all(|r| r.member == "alice"), "{mine:?}");
    let bobs = g.my_refusals(&bob(), 10).unwrap();
    assert_eq!(bobs.len(), 2);
    assert!(bobs.iter().all(|r| r.member == "bob"), "{bobs:?}");
    assert!(
        g.my_refusals(&Principal::unattributed("carol"), 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn the_limit_is_honoured_at_zero_one_the_count_and_past_it() {
    let (g, t) = vault();
    for i in 0..3 {
        tick(&t);
        let _ = g.post(&alice(), "vault", &format!("try {i}")).unwrap_err();
    }
    assert_eq!(g.my_refusals(&alice(), 0).unwrap().len(), 0);
    let one = g.my_refusals(&alice(), 1).unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].ts, t.get(), "the newest comes first");
    assert_eq!(g.my_refusals(&alice(), 2).unwrap().len(), 2);
    assert_eq!(g.my_refusals(&alice(), 3).unwrap().len(), 3);
    assert_eq!(g.my_refusals(&alice(), 4).unwrap().len(), 3);
}

#[test]
fn the_limit_counts_only_the_callers_own_refusals() {
    let (g, t) = vault();
    let _ = g.post(&alice(), "vault", "mine").unwrap_err();
    for i in 0..3 {
        tick(&t);
        let _ = g.post(&bob(), "vault", &format!("his {i}")).unwrap_err();
    }
    let mine = g.my_refusals(&alice(), 1).unwrap();
    assert_eq!(
        mine.len(),
        1,
        "bob's newer refusals must not use up the limit"
    );
    assert_eq!(mine[0].member, "alice");
}

#[test]
fn the_list_holds_no_refused_text_and_no_other_members_name() {
    let (g, _) = vault();
    let _ = g
        .post(&alice(), "town", &format!("secret {TOKEN}"))
        .unwrap_err();
    let _ = g.post(&bob(), "vault", "bobs-private-words").unwrap_err();
    let shown = format!("{:?}", g.my_refusals(&alice(), 10).unwrap());
    assert!(
        !shown.contains(TOKEN) && !shown.contains("abcdefgh"),
        "{shown}"
    );
    assert!(
        !shown.contains("bob") && !shown.contains("private-words"),
        "{shown}"
    );
}

#[test]
fn listing_is_not_itself_a_refusal_and_records_nothing() {
    let (g, _) = vault();
    let _ = g.post(&alice(), "vault", "once").unwrap_err();
    let _ = g.my_refusals(&alice(), 10).unwrap();
    let _ = g.my_refusals(&alice(), 10).unwrap();
    assert_eq!(g.admin_refusals(&admin(), 10).unwrap().len(), 1);
}

#[test]
fn a_member_stopped_by_the_breaker_can_still_read_why() {
    let (g, t) = vault();
    for i in 0..5 {
        tick(&t);
        let _ = g.post(&bob(), "vault", &format!("try {i}")).unwrap_err();
    }
    assert_eq!(
        rule_of(g.post(&bob(), "town", "innocent").unwrap_err()),
        "breaker"
    );
    let mine = g.my_refusals(&bob(), 10).unwrap();
    assert!(mine.len() >= 5, "{mine:?}");
}

#[test]
fn a_bad_member_name_is_refused() {
    let (g, _) = vault();
    let bad = Principal::unattributed("not a name!");
    assert!(g.my_refusals(&bad, 10).is_err());
}

#[test]
fn an_ordinary_member_of_a_members_gateway_may_list_its_own_and_still_not_the_admin_record() {
    let (g, _) = vault();
    let g = g.for_members();
    let _ = g.post(&alice(), "vault", "once").unwrap_err();
    assert_eq!(g.my_refusals(&alice(), 10).unwrap().len(), 1);
    assert!(is_refused(g.admin_refusals(&alice(), 10)));
}
