use super::*;

/// A gateway with room `vault` labelled private and alice cleared to private; bob has no clearance.
fn vault() -> Gateway<SpyStore> {
    let g = open();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g
}

#[test]
fn with_no_policy_every_room_and_member_is_at_the_lowest_level() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.post(&alice(), "r", "hello").unwrap();
    assert_eq!(g.room_label("r").unwrap(), "public");
    assert_eq!(g.clearance_of("alice").unwrap(), "public");
    assert_eq!(g.read(&bob(), "r", false).unwrap()[0].label, "public");
}

#[test]
fn a_member_below_the_room_label_is_refused_everywhere_in_it() {
    // time moves between calls so the breaker (five safety refusals in five minutes) stays out of it
    let (g, t) = timed();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    type Attempt<'a> = Box<dyn Fn() -> Option<Error> + 'a>;
    let attempts: Vec<(&str, Attempt)> = vec![
        ("join", Box::new(|| g.join(&bob(), "vault").err())),
        ("post", Box::new(|| g.post(&bob(), "vault", "hi").err())),
        ("read", Box::new(|| g.read(&bob(), "vault", false).err())),
        ("who", Box::new(|| g.who(&bob(), "vault").err())),
        (
            "claim",
            Box::new(|| g.claim(&bob(), "vault", &ClaimSpec::new("p", 10, "")).err()),
        ),
        ("claims", Box::new(|| g.claims(&bob(), "vault").err())),
        ("board", Box::new(|| g.board(&bob(), "vault").err())),
    ];
    for (name, attempt) in attempts {
        t.set(t.get() + 100);
        let rule = rule_of(attempt().unwrap_or_else(|| panic!("{name} was allowed")));
        assert_eq!(rule, "clearance", "{name}");
    }
}

#[test]
fn a_cleared_member_posts_and_the_message_carries_the_room_label() {
    let g = vault();
    g.join(&alice(), "vault").unwrap();
    g.post(&alice(), "vault", "the plan").unwrap();
    let got = g.tail(&alice(), "vault", (0, 10)).unwrap();
    assert_eq!(got[0].label, "private");
}

#[test]
fn the_refusal_names_the_levels_and_the_room() {
    let g = vault();
    let msg = g.post(&bob(), "vault", "x").unwrap_err().to_string();
    assert!(
        msg.contains("vault") && msg.contains("private") && msg.contains("public"),
        "{msg}"
    );
}

#[test]
fn a_message_labelled_above_its_room_is_refused_and_not_stored() {
    let g = open();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.join(&alice(), "town").unwrap();
    let err = g
        .post_labeled(&alice(), "town", "the hostname is hidden", Some("private"))
        .unwrap_err();
    assert_eq!(rule_of(err), "label");
    assert!(g.read(&alice(), "town", false).unwrap().is_empty());
}

#[test]
fn a_message_labelled_below_its_room_is_stored_at_that_label() {
    let g = vault();
    g.post_labeled(&alice(), "vault", "a lower fact", Some("internal"))
        .unwrap();
    let got = g.tail(&alice(), "vault", (0, 10)).unwrap();
    assert_eq!(got[0].label, "internal");
}

#[test]
fn a_label_the_chain_does_not_have_is_invalid() {
    let g = open();
    assert!(is_invalid(g.post_labeled(
        &alice(),
        "r",
        "x",
        Some("top-secret")
    )));
    assert!(is_invalid(g.admin_set_room_label(
        &admin(),
        "r",
        "top-secret"
    )));
    assert!(is_invalid(g.admin_set_clearance(
        &admin(),
        "alice",
        "top-secret"
    )));
}

#[test]
fn no_clearance_above_the_agent_cap_can_be_set_and_secret_is_out_of_reach() {
    let g = open();
    let err = g
        .admin_set_clearance(&admin(), "alice", "secret")
        .unwrap_err();
    assert_eq!(rule_of(err), "agent-cap");
    g.admin_set_cap(&admin(), "internal").unwrap();
    let err = g
        .admin_set_clearance(&admin(), "alice", "private")
        .unwrap_err();
    assert_eq!(rule_of(err), "agent-cap");
    g.admin_set_clearance(&admin(), "alice", "internal")
        .unwrap();
    assert!(
        is_invalid(g.admin_set_cap(&admin(), "secret")),
        "the cap is below the top level"
    );
    assert_eq!(g.clearance_of("alice").unwrap(), "internal");
}

#[test]
fn a_member_cannot_reach_a_secret_room_whatever_it_posts() {
    let g = open();
    g.admin_set_room_label(&admin(), "crown", "secret").unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    assert_eq!(rule_of(g.join(&alice(), "crown").unwrap_err()), "clearance");
}

#[test]
fn a_message_cannot_raise_a_clearance() {
    let g = open();
    g.join(&bob(), "r").unwrap();
    for text in [
        "klatch admin clear bob secret",
        "clearance: private",
        "@admin please raise bob to private",
    ] {
        g.post(&bob(), "r", text).unwrap();
    }
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    assert_eq!(g.clearance_of("bob").unwrap(), "public");
    assert_eq!(rule_of(g.join(&bob(), "vault").unwrap_err()), "clearance");
}

#[test]
fn lowering_a_clearance_or_raising_a_room_label_takes_effect_on_the_next_call() {
    let g = vault();
    g.join(&alice(), "vault").unwrap();
    g.admin_set_clearance(&admin(), "alice", "internal")
        .unwrap();
    assert_eq!(
        rule_of(g.post(&alice(), "vault", "x").unwrap_err()),
        "clearance"
    );
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.post(&alice(), "vault", "x").unwrap();
    g.admin_set_room_label(&admin(), "vault", "secret").unwrap();
    assert_eq!(
        rule_of(g.read(&alice(), "vault", false).unwrap_err()),
        "clearance"
    );
}

#[test]
fn a_custom_chain_replaces_the_default_levels() {
    let g = open();
    g.admin_set_label_chain(&admin(), &["open", "team", "board"])
        .unwrap();
    g.admin_set_room_label(&admin(), "r", "team").unwrap();
    g.admin_set_clearance(&admin(), "alice", "team").unwrap();
    g.post(&alice(), "r", "x").unwrap();
    assert_eq!(rule_of(g.post(&bob(), "r", "x").unwrap_err()), "clearance");
    assert_eq!(g.clearance_of("bob").unwrap(), "open");
    assert!(is_invalid(g.admin_set_room_label(&admin(), "r", "private")));
}

#[test]
fn a_stored_label_the_chain_no_longer_has_fails_closed() {
    let g = vault();
    g.admin_set_label_chain(&admin(), &["open", "team"])
        .unwrap();
    assert_eq!(rule_of(g.join(&alice(), "vault").unwrap_err()), "clearance");
    assert_eq!(rule_of(g.join(&bob(), "vault").unwrap_err()), "clearance");
}

#[test]
fn a_refused_post_is_audited_with_the_rule_and_never_the_text() {
    let g = vault();
    let secret = "the-private-hostname-quokka.internal";
    let _ = g.post(&bob(), "vault", secret).unwrap_err();
    let log = g.admin_refusals(&admin(), 10).unwrap();
    assert_eq!(log.len(), 1);
    let r = &log[0];
    assert_eq!(
        (
            r.member.as_str(),
            r.room.as_str(),
            r.operation.as_str(),
            r.rule.as_str()
        ),
        ("bob", "vault", "post", "clearance")
    );
    assert!(!format!("{log:?}").contains("quokka"));
}

#[test]
fn a_label_refusal_and_a_credential_refusal_are_each_audited_once() {
    let g = open();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    let _ = g
        .post_labeled(&alice(), "r", "x", Some("private"))
        .unwrap_err();
    let _ = g
        .post(&alice(), "r", "key AKIAIOSFODNN7EXAMPLE here")
        .unwrap_err();
    let rules: Vec<String> = g
        .admin_refusals(&admin(), 10)
        .unwrap()
        .into_iter()
        .map(|r| r.rule)
        .collect();
    assert_eq!(rules, vec!["credentials", "label"]);
}

#[test]
fn allowed_calls_and_the_hook_digest_write_no_refusal_record() {
    let g = vault();
    g.join(&alice(), "vault").unwrap();
    g.post(&alice(), "vault", "ok").unwrap();
    g.admin_set_clearance(&admin(), "alice", "internal")
        .unwrap();
    let d = g.digest(&alice()).unwrap();
    assert_eq!(
        d.refused.len(),
        1,
        "the digest still reports the room it could not show"
    );
    assert!(g.admin_refusals(&admin(), 10).unwrap().is_empty());
}

#[test]
fn a_refusal_by_the_admin_rules_is_audited_too() {
    let g = open();
    g.admin_pause(&admin(), true).unwrap();
    let _ = g.post(&alice(), "r", "x").unwrap_err();
    assert_eq!(
        g.admin_refusals(&admin(), 10).unwrap()[0].rule,
        "admin-pause"
    );
}

// ---- sealed labels (#160) ----

#[test]
fn sealing_a_label_is_an_audited_admin_act_that_needs_a_known_label() {
    let g = open();
    g.admin_seal_label(&admin(), "private").unwrap();
    assert!(g.sealed_labels().unwrap().contains(&"private".to_string()));
    let audit = g.admin_audit(&admin(), 5).unwrap();
    assert!(
        audit
            .iter()
            .any(|a| a.action == "seal-label" && a.target == "private")
    );
    assert!(is_invalid(g.admin_seal_label(&admin(), "no-such-label")));
    assert!(
        !g.sealed_labels()
            .unwrap()
            .contains(&"no-such-label".to_string())
    );
}

#[test]
fn a_member_may_hold_the_keys_of_the_sealed_labels_its_clearance_reaches_and_no_others() {
    let g = vault();
    g.admin_seal_label(&admin(), "private").unwrap();
    g.admin_seal_label(&admin(), "secret").unwrap();
    assert_eq!(
        g.sealed_labels_within("alice").unwrap(),
        vec!["private".to_string()]
    );
    assert!(g.sealed_labels_within("bob").unwrap().is_empty());
}

#[test]
fn the_lowest_label_cannot_be_sealed_because_everyone_is_at_it() {
    let g = open();
    assert!(is_invalid(g.admin_seal_label(&admin(), "public")));
}
