use super::*;

#[test]
fn a_policy_value_is_set_read_replaced_and_cleared() {
    let s = store();
    assert_eq!(s.policy_get("label:r").unwrap(), None);
    s.policy_set("label:r", Some("private"), 1).unwrap();
    assert_eq!(s.policy_get("label:r").unwrap().as_deref(), Some("private"));
    s.policy_set("label:r", Some("secret"), 2).unwrap();
    assert_eq!(s.policy_get("label:r").unwrap().as_deref(), Some("secret"));
    s.policy_set("label:r", None, 3).unwrap();
    assert_eq!(s.policy_get("label:r").unwrap(), None);
}

#[test]
fn policy_entries_are_listed_by_prefix_in_key_order() {
    let s = store();
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

#[test]
fn a_prefix_with_wildcard_characters_matches_literally() {
    let s = store();
    s.policy_set("a%b:x", Some("1"), 1).unwrap();
    s.policy_set("azzb:x", Some("2"), 1).unwrap();
    assert_eq!(s.policy_list("a%b").unwrap().len(), 1);
}

#[test]
fn refusals_are_kept_newest_first_with_the_rule_and_no_text() {
    let s = store();
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

#[test]
fn the_rules_a_member_was_refused_under_since_a_time_are_counted_per_member() {
    let s = store();
    for (m, t) in [("alice", 5), ("alice", 50), ("alice", 60), ("bob", 55)] {
        s.record_refusal(&NewRefusal::new(m, "r", "post", "label"), t)
            .unwrap();
    }
    assert_eq!(s.refusal_rules_since("alice", 40).unwrap().len(), 2);
    assert_eq!(s.refusal_rules_since("bob", 40).unwrap(), vec!["label"]);
    assert!(s.refusal_rules_since("carol", 0).unwrap().is_empty());
}
