//! Every safety refusal ends with the step that fixes it (#516, #678). A hint is text a member sees, so it must
//! never carry the refused text, the label of a room the member cannot see, or anything of another member.

use super::*;

const TOKEN: &str = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";

fn hint_of(e: &Error) -> String {
    e.next_step()
        .unwrap_or_else(|| panic!("no next step in: {e}"))
        .to_string()
}

#[test]
fn a_clearance_refusal_says_to_ask_the_admin_and_names_no_label() {
    let g = open();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    let e = g.join(&bob(), "vault").unwrap_err();
    assert_eq!(rule_of_ref(&e), "clearance");
    let hint = hint_of(&e);
    assert_eq!(
        hint,
        "ask the admin to raise your clearance or to relabel the room"
    );
    assert!(
        !hint.contains("private") && !hint.contains("vault"),
        "{hint}"
    );
}

#[test]
fn a_label_refusal_names_the_release_request_and_a_room_labelled_high_enough() {
    let g = open();
    g.admin_set_room_label(&admin(), "town", "public").unwrap();
    g.admin_set_clearance(&admin(), "alice", "internal")
        .unwrap();
    let e = g
        .post_labeled(&alice(), ("town", "a private fact", Some("internal")))
        .unwrap_err();
    assert_eq!(rule_of_ref(&e), "label");
    assert_eq!(
        hint_of(&e),
        "post it to a room labelled internal or higher; to release a message already posted, run `klatch cross SOURCE SEQ --to town`"
    );
}

#[test]
fn a_scrub_refusal_says_to_remove_or_reword_the_match_and_never_prints_the_term() {
    let g = open();
    g.admin_add_scrub_term(&admin(), "hosts", "quokka.internal")
        .unwrap();
    let e = g
        .post(&alice(), "r", "deploy to quokka.internal now")
        .unwrap_err();
    assert_eq!(rule_of_ref(&e), "scrub:hosts");
    let hint = hint_of(&e);
    assert_eq!(
        hint,
        "remove or reword the part that matches the rule hosts, then post again"
    );
    assert!(!hint.to_lowercase().contains("quokka"), "{hint}");
}

#[test]
fn a_credential_refusal_says_to_remove_the_secret_and_never_prints_it() {
    let g = open();
    let e = g
        .post(&alice(), "r", &format!("use this: {TOKEN}"))
        .unwrap_err();
    assert_eq!(rule_of_ref(&e), "credentials");
    let hint = hint_of(&e);
    assert_eq!(
        hint,
        "remove the secret and say where it is kept instead, then post again"
    );
    assert!(
        !hint.contains(TOKEN) && !hint.contains("abcdefgh"),
        "{hint}"
    );
}

#[test]
fn a_canary_refusal_says_to_tell_an_admin_and_names_neither_text_nor_home_room() {
    let g = open();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.admin_plant_canary(&admin(), "vault").unwrap();
    let body = g.tail(&alice(), "vault", (0, 10)).unwrap()[0].body.clone();
    let token = body.rsplit(' ').next().unwrap().to_string();
    let e = g
        .post(&alice(), "town", &format!("fyi {token}"))
        .unwrap_err();
    assert_eq!(rule_of_ref(&e), "canary");
    let hint = hint_of(&e);
    assert_eq!(
        hint,
        "do not post it again; ask an admin to look and reopen room town"
    );
    assert!(!hint.contains(&token) && !hint.contains("vault"), "{hint}");
}

#[test]
fn a_hint_is_the_end_of_the_reason_and_the_rule_is_unchanged() {
    let g = open();
    let e = g
        .post(&alice(), "r", &format!("use this: {TOKEN}"))
        .unwrap_err();
    let Error::Refused { rule, reason } = &e else {
        panic!("not a refusal: {e}");
    };
    assert_eq!(rule, "credentials");
    assert!(reason.ends_with(&hint_of(&e)), "{reason}");
    assert!(reason.contains("; next step: "), "{reason}");
}

#[test]
fn a_refusal_without_a_safety_rule_is_left_as_it_was() {
    let g = open();
    g.admin_pause(&admin(), true).unwrap();
    let e = g.post(&alice(), "r", "hi").unwrap_err();
    assert_eq!(e.next_step(), None, "{e}");
}

fn rule_of_ref(e: &Error) -> &str {
    match e {
        Error::Refused { rule, .. } => rule,
        other => panic!("not a refusal: {other}"),
    }
}
