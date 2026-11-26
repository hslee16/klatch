use super::*;
pub fn a_policy_value_is_set_read_replaced_and_cleared<S: Store>(make: fn() -> S) {
    let s = make();
    assert_eq!(s.policy_get("label:r").unwrap(), None);
    s.policy_set("label:r", Some("private"), 1).unwrap();
    assert_eq!(s.policy_get("label:r").unwrap().as_deref(), Some("private"));
    s.policy_set("label:r", Some("secret"), 2).unwrap();
    assert_eq!(s.policy_get("label:r").unwrap().as_deref(), Some("secret"));
    s.policy_set("label:r", None, 3).unwrap();
    assert_eq!(s.policy_get("label:r").unwrap(), None);
}
pub fn policy_entries_are_listed_by_prefix_in_key_order<S: Store>(make: fn() -> S) {
    let s = make();
    s.policy_set("scrub:b", Some("two"), 1).unwrap();
    s.policy_set("scrub:a", Some("one"), 1).unwrap();
    s.policy_set("label:r", Some("x"), 1).unwrap();
    assert_eq!(
        s.policy_list("scrub:").unwrap(),
        vec![
            ("scrub:a".to_string(), "one".to_string()),
            ("scrub:b".to_string(), "two".to_string())
        ]
    );
}
pub fn a_prefix_with_wildcard_characters_matches_literally<S: Store>(make: fn() -> S) {
    let s = make();
    s.policy_set("a%b:x", Some("1"), 1).unwrap();
    s.policy_set("azzb:x", Some("2"), 1).unwrap();
    assert_eq!(s.policy_list("a%b").unwrap().len(), 1);
}
pub fn refusals_are_kept_newest_first_with_the_rule_and_no_text<S: Store>(make: fn() -> S) {
    let s = make();
    s.record_refusal(&NewRefusal::new("alice", "r", "post", "label"), 10)
        .unwrap();
    s.record_refusal(&NewRefusal::new("bob", "q", "claim", "credentials"), 20)
        .unwrap();
    let log = s.refusals(10).unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(
        (log[0].member.as_str(), log[0].rule.as_str(), log[0].ts),
        ("bob", "credentials", 20)
    );
    assert_eq!(
        (log[1].room.as_str(), log[1].operation.as_str()),
        ("r", "post")
    );
    assert_eq!(s.refusals(1).unwrap().len(), 1);
}
pub fn one_members_refusals_are_listed_newest_first_and_the_limit_counts_only_theirs<S: Store>(
    make: fn() -> S,
) {
    let s = make();
    s.record_refusal(&NewRefusal::new("alice", "r", "post", "label"), 10)
        .unwrap();
    s.record_refusal(&NewRefusal::new("bob", "q", "claim", "credentials"), 20)
        .unwrap();
    s.record_refusal(&NewRefusal::new("alice", "t", "status", "scrub:hosts"), 30)
        .unwrap();
    s.record_refusal(&NewRefusal::new("bob", "q", "post", "canary"), 40)
        .unwrap();
    let mine = s.refusals_of("alice", 10).unwrap();
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
            ("alice", "t", "status", "scrub:hosts", 30),
            ("alice", "r", "post", "label", 10)
        ]
    );
    assert_eq!(s.refusals_of("alice", 1).unwrap().len(), 1);
    assert_eq!(s.refusals_of("alice", 1).unwrap()[0].ts, 30);
    assert!(s.refusals_of("alice", 0).unwrap().is_empty());
    assert!(s.refusals_of("carol", 10).unwrap().is_empty());
}
pub fn the_rules_a_member_was_refused_under_since_a_time_are_counted_per_member<S: Store>(
    make: fn() -> S,
) {
    let s = make();
    for (m, t) in [("alice", 5), ("alice", 50), ("alice", 60), ("bob", 55)] {
        s.record_refusal(&NewRefusal::new(m, "r", "post", "label"), t)
            .unwrap();
    }
    assert_eq!(s.refusal_rules_since("alice", 40).unwrap().len(), 2);
    assert_eq!(s.refusal_rules_since("bob", 40).unwrap(), vec!["label"]);
    assert!(s.refusal_rules_since("carol", 0).unwrap().is_empty());
}
pub fn the_audit_page_filters_before_the_limit_leaves_out_the_note_and_counts_what_it_hid<
    S: Store,
>(
    make: fn() -> S,
) {
    use crate::store::FORWARDED_IGNORED_ACTION as NOTE;
    let s = make();
    for (i, action) in ["pause", "resume", "close"].into_iter().enumerate() {
        s.record_audit(&NewAudit::new(action, "t", ""), 10 + i as i64)
            .unwrap();
    }
    for i in 0..5 {
        s.record_audit(&NewAudit::new(NOTE, "", "requests=1"), 100 + i)
            .unwrap();
    }
    let page = s.audit_page(2, None).unwrap();
    let seen: Vec<&str> = page.rows.iter().map(|r| r.action.as_str()).collect();
    assert_eq!(
        seen,
        ["close", "resume"],
        "the notes are newer, and not shown"
    );
    assert_eq!(page.hidden, 5);
    let all = s.audit_page(10, None).unwrap();
    assert_eq!(all.rows.len(), 3);
    assert_eq!(all.hidden, 5);
    let notes = s.audit_page(3, Some(NOTE)).unwrap();
    assert_eq!(notes.rows.len(), 3);
    assert!(notes.rows.iter().all(|r| r.action == NOTE));
    assert_eq!(
        notes.hidden, 0,
        "nothing is hidden when the note is asked for"
    );
    let one = s.audit_page(1, Some("pause")).unwrap();
    assert_eq!(one.rows.len(), 1, "the filter is applied before the limit");
    assert_eq!(one.rows[0].ts, 10);
    assert!(
        s.audit_page(5, Some("none-of-this"))
            .unwrap()
            .rows
            .is_empty()
    );
    assert!(s.audit_page(0, None).unwrap().rows.is_empty());
}
