use super::*;

fn with_term() -> Gateway<SpyStore> {
    let g = open();
    g.admin_add_scrub_term(&admin(), "hosts", "quokka.internal")
        .unwrap();
    g
}

#[test]
fn a_term_refuses_a_post_naming_the_rule_and_never_the_term() {
    let g = with_term();
    let err = g
        .post(&alice(), "r", "deploy to Quokka.Internal now")
        .unwrap_err();
    let msg = err.to_string();
    assert_eq!(rule_of(err), "scrub:hosts");
    assert!(!msg.to_lowercase().contains("quokka"), "{msg}");
    assert!(msg.contains("hosts"), "{msg}");
    assert!(g.tail(&alice(), "r", (0, 10)).unwrap().is_empty());
}

#[test]
fn text_without_a_term_passes() {
    let g = with_term();
    g.post(&alice(), "r", "a quokka is a small wallaby")
        .unwrap();
}

#[test]
fn a_term_also_refuses_a_claim_reason_a_release_note_and_a_status() {
    let g = with_term();
    let who = alice();
    assert_eq!(
        rule_of(
            g.claim(&who, "r", &ClaimSpec::new("p", 60, "on quokka.internal"))
                .unwrap_err()
        ),
        "scrub:hosts"
    );
    g.claim(&who, "r", &ClaimSpec::new("p", 60, "")).unwrap();
    assert_eq!(
        rule_of(
            g.release_with_note(&who, ("r", "p"), "moved to quokka.internal")
                .unwrap_err()
        ),
        "scrub:hosts"
    );
    assert_eq!(
        rule_of(g.set_status(&who, "r", "on quokka.internal").unwrap_err()),
        "scrub:hosts"
    );
}

#[test]
fn a_room_at_or_above_private_only_refuses_credentials() {
    let g = with_term();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.post(&alice(), "vault", "quokka.internal is the host")
        .unwrap();
    assert_eq!(
        rule_of(
            g.post(&alice(), "vault", "key AKIAIOSFODNN7EXAMPLE")
                .unwrap_err()
        ),
        "credentials"
    );
    g.admin_set_room_label(&admin(), "inner", "internal")
        .unwrap();
    g.admin_set_clearance(&admin(), "bob", "internal").unwrap();
    assert_eq!(
        rule_of(g.post(&bob(), "inner", "quokka.internal").unwrap_err()),
        "scrub:hosts"
    );
}

#[test]
fn removing_a_term_lifts_it_and_the_list_shows_what_is_set() {
    let g = with_term();
    g.admin_add_scrub_term(&admin(), "codename", "Project Falcon")
        .unwrap();
    let list = g.admin_scrub_terms(&admin()).unwrap();
    assert_eq!(
        list,
        vec![
            ("codename".to_string(), "Project Falcon".to_string()),
            ("hosts".to_string(), "quokka.internal".to_string())
        ]
    );
    g.admin_remove_scrub_term(&admin(), "hosts").unwrap();
    g.post(&alice(), "r", "quokka.internal").unwrap();
    assert!(g.post(&alice(), "r", "project falcon").is_err());
    assert!(matches!(
        g.admin_remove_scrub_term(&admin(), "hosts"),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn a_scrub_refusal_is_audited_with_the_rule_and_not_the_text() {
    let g = with_term();
    let _ = g
        .post(&alice(), "r", "quokka.internal is the host")
        .unwrap_err();
    let log = g.admin_refusals(&admin(), 5).unwrap();
    assert_eq!(log[0].rule, "scrub:hosts");
    assert!(!format!("{log:?}").contains("quokka"));
}

#[test]
fn a_bad_rule_name_or_term_is_invalid() {
    let g = open();
    for (name, term) in [
        ("bad name", "x"),
        ("", "x"),
        ("ok", ""),
        ("ok", "  "),
        ("ok", "a\nb"),
    ] {
        assert!(
            is_invalid(g.admin_add_scrub_term(&admin(), name, term)),
            "{name:?} {term:?}"
        );
    }
    assert!(is_invalid(g.admin_add_scrub_term(
        &admin(),
        "ok",
        &"t".repeat(201)
    )));
}
