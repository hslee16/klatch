//! References an ask carries to artifacts outside klatch (#1318): the gateway checks them before anything is posted,
//! stores them beside the ask, and shows them wherever the ask is shown. klatch never fetches or resolves one.

use super::*;
use crate::domain::AskRef;

fn r(kind: &str, value: &str) -> AskRef {
    AskRef {
        kind: kind.into(),
        value: value.into(),
        ..Default::default()
    }
}

fn ask(g: &Gateway<SpyStore>, refs: &[AskRef]) -> Result<crate::domain::Posted> {
    g.post_ask(&alice(), ("r", "seeking: a thing", None), None, refs)
}

fn nothing_posted(g: &Gateway<SpyStore>) -> bool {
    g.read(&bob(), "r", false).unwrap().is_empty()
}

#[test]
fn refs_are_stored_with_the_ask_and_shown_by_open_asks_and_read() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    let mut log = r("log", "ci.log");
    log.label = Some("nightly".into());
    log.snippet = Some("segfault in parser".into());
    let seq = ask(&g, &[r("pr", "o/r#12"), log.clone()]).unwrap().seq;
    let plain = g.post(&alice(), "r", "seeking: plain").unwrap().seq;
    let asks = g.open_asks(&alice(), "r").unwrap();
    let refs = |n: i64| asks.iter().find(|a| a.seq == n).unwrap().refs.clone();
    assert_eq!(refs(seq), vec![r("pr", "o/r#12"), log.clone()]);
    assert!(refs(plain).is_empty());
    let seen = g.read(&bob(), "r", false).unwrap();
    let on = |n: i64| seen.iter().find(|m| m.seq == n).unwrap().refs.clone();
    assert_eq!(on(seq), vec![r("pr", "o/r#12"), log]);
    assert!(on(plain).is_empty());
}

#[test]
fn an_unknown_kind_is_accepted_as_opaque_text() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    ask(&g, &[r("figma-frame", "page 3, frame \"login\"")]).unwrap();
    assert_eq!(
        g.open_asks(&alice(), "r").unwrap()[0].refs[0].kind,
        "figma-frame"
    );
}

#[test]
fn an_ask_with_no_refs_posts_as_before() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    ask(&g, &[]).unwrap();
    assert!(g.open_asks(&alice(), "r").unwrap()[0].refs.is_empty());
}

#[test]
fn caps_are_refused_and_nothing_is_posted_or_cut() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    let many: Vec<AskRef> = (0..33).map(|i| r("other", &format!("v{i}"))).collect();
    assert!(is_invalid(ask(&g, &many)));
    let mut big = r("log", "x");
    big.snippet = Some("s".repeat(crate::domain::MAX_SNIPPET_BYTES + 1));
    assert!(is_invalid(ask(&g, &[big])));
    assert!(nothing_posted(&g));
}

#[test]
fn refs_go_only_on_an_ask() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    assert!(is_invalid(g.post_ask(
        &alice(),
        ("r", "just talking", None),
        None,
        &[r("pr", "#1")]
    )));
    assert!(nothing_posted(&g));
}

#[test]
fn a_credential_in_a_url_a_value_a_label_or_a_snippet_is_refused_by_the_credential_rule() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    let token = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let mut in_label = r("other", "x");
    in_label.label = Some(format!("key {token}"));
    let mut in_snippet = r("log", "x");
    in_snippet.snippet = Some(format!("export TOKEN={token}"));
    for refs in [
        vec![r("url", "https://user:pw@example.com/x")],
        vec![r("url", "https://example.com/x?access_token=abc")],
        vec![r("other", token)],
        vec![in_label],
        vec![in_snippet],
    ] {
        let err = ask(&g, &refs).unwrap_err();
        let msg = err.to_string();
        assert!(
            rule_of(err).starts_with("credential"),
            "{refs:?}: not the credential rule"
        );
        assert!(!msg.contains(token) && !msg.contains("pw@"), "{msg}");
    }
    assert!(nothing_posted(&g));
}

#[test]
fn a_private_term_in_a_ref_is_refused_like_message_text() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.admin_add_scrub_term(&admin(), "hosts", "quokka.internal")
        .unwrap();
    let mut snippet = r("log", "x");
    snippet.snippet = Some("connecting to Quokka.Internal failed".into());
    assert_eq!(rule_of(ask(&g, &[snippet]).unwrap_err()), "scrub:hosts");
    assert_eq!(
        rule_of(ask(&g, &[r("host", "quokka.internal")]).unwrap_err()),
        "scrub:hosts"
    );
    assert!(nothing_posted(&g));
}

#[test]
fn a_ref_cannot_launder_a_restricted_label() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    // a public ask may not point at a room above its own label, by room or by ask
    assert_eq!(
        rule_of(ask(&g, &[r("room", "vault")]).unwrap_err()),
        "label"
    );
    assert_eq!(
        rule_of(ask(&g, &[r("ask", "vault#3")]).unwrap_err()),
        "label"
    );
    // a room at or below its own label is fine
    ask(&g, &[r("room", "r"), r("ask", "r#1"), r("ask", "7")]).unwrap();
    // an ask in the vault may point at the vault
    g.join(&alice(), "vault").unwrap();
    g.post_ask(
        &alice(),
        ("vault", "seeking: x", None),
        None,
        &[r("room", "vault")],
    )
    .unwrap();
    assert_eq!(g.read(&bob(), "r", false).unwrap().len(), 1);
}

#[test]
fn refs_are_not_stored_beside_text_under_a_sealed_label() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.admin_set_room_label(&admin(), "r", "internal").unwrap();
    g.admin_set_clearance(&admin(), "alice", "internal")
        .unwrap();
    g.admin_seal_label_keep_plain(&admin(), "internal").unwrap();
    assert_eq!(rule_of(ask(&g, &[r("pr", "#1")]).unwrap_err()), "label");
}

#[test]
fn refs_come_from_the_record_never_from_the_text_of_the_ask() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    let seq = g
        .post(&alice(), "r", "seeking: x [refs pr #99]")
        .unwrap()
        .seq;
    let asks = g.open_asks(&alice(), "r").unwrap();
    assert!(asks.iter().find(|a| a.seq == seq).unwrap().refs.is_empty());
}

#[test]
fn refs_and_a_scope_go_together_and_a_ref_implies_no_scope() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    let with_both = g
        .post_ask(
            &alice(),
            ("r", "seeking: a", None),
            Some("git:foo"),
            &[r("pr", "#1")],
        )
        .unwrap()
        .seq;
    let refs_only = ask(&g, &[r("path", "src/lib.rs")]).unwrap().seq;
    let asks = g.open_asks(&alice(), "r").unwrap();
    let one = |n: i64| asks.iter().find(|a| a.seq == n).unwrap();
    assert_eq!(one(with_both).scope.as_deref(), Some("git:foo"));
    assert_eq!(one(refs_only).scope, None);
}

#[test]
fn could_answer_shows_the_refs_of_the_ask_and_changes_no_verdict() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    let mut with_snippet = r("log", "ci.log");
    with_snippet.snippet = Some("SECRET-LOOKING-OUTPUT".into());
    let seq = ask(&g, &[r("pr", "#1"), with_snippet]).unwrap().seq;
    let c = g.could_answer(&admin(), "r", "helper", seq).unwrap();
    assert!(c.could(), "{c:?}");
    assert_eq!(
        c.refs.iter().map(|x| x.kind.as_str()).collect::<Vec<_>>(),
        ["pr", "log"]
    );
    // the check carries no snippet text, only kind, value and label
    assert!(c.refs.iter().all(|x| x.snippet.is_none()), "{c:?}");
}

#[test]
fn a_non_member_cannot_post_an_ask_with_refs() {
    let g = open();
    let outsider = Principal::unattributed("bad name");
    assert!(is_invalid(g.post_ask(
        &outsider,
        ("r", "seeking: x", None),
        None,
        &[r("pr", "#1")]
    )));
}

#[test]
fn a_message_ref_must_name_a_message_in_a_room_the_ask_label_covers() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.join(&alice(), "vault").unwrap();
    let high = g.post(&alice(), "vault", "hidden").unwrap().id;
    let low = g.post(&alice(), "r", "visible").unwrap().id;
    // a public ask may not point at a message above its own label, and the refusal is the room ref's
    let e = ask(&g, &[r("message", &high)]).unwrap_err();
    assert_eq!(rule_of(e), "label");
    // the visible message was posted to "r", so only it is there and no ask was
    assert_eq!(g.read(&bob(), "r", false).unwrap().len(), 1);
    // a message at or below the label is fine
    ask(&g, &[r("message", &low)]).unwrap();
    // an ask in the vault may point at a vault message
    g.post_ask(
        &alice(),
        ("vault", "seeking: x", None),
        None,
        &[r("message", &high)],
    )
    .unwrap();
}

#[test]
fn a_message_ref_to_an_unknown_id_behaves_as_a_ref_to_an_unknown_room() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    // accepted, exactly as a room that does not exist is (it counts as the lowest label), so a refusal never
    // tells an asker which ids exist
    ask(&g, &[r("message", "no-such-id"), r("room", "no-such-room")]).unwrap();
}

#[test]
fn a_message_ref_refusal_is_the_same_whether_the_message_is_above_the_label_or_the_room_is() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.join(&alice(), "vault").unwrap();
    let high = g.post(&alice(), "vault", "hidden").unwrap().id;
    let by_message = ask(&g, &[r("message", &high)]).unwrap_err();
    let by_room = ask(&g, &[r("room", "vault")]).unwrap_err();
    assert_eq!(format!("{by_message:?}"), format!("{by_room:?}"));
    assert!(!format!("{by_message:?}").contains(&high));
}

#[test]
fn a_lowercased_message_id_above_the_label_is_still_refused() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.join(&alice(), "vault").unwrap();
    let high = g.post(&alice(), "vault", "hidden").unwrap().id;
    assert_ne!(high, high.to_lowercase(), "the id has letters to fold");
    let e = ask(&g, &[r("message", &high.to_lowercase())]).unwrap_err();
    assert_eq!(rule_of(e), "label");
    assert!(nothing_posted(&g));
}

#[test]
fn a_failing_message_lookup_fails_the_post_closed() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    let low = g.post(&alice(), "r", "visible").unwrap().id;
    g.store.fail_message_room.set(true);
    assert!(ask(&g, &[r("message", &low)]).is_err());
    g.store.fail_message_room.set(false);
    assert_eq!(g.read(&bob(), "r", false).unwrap().len(), 1);
}

#[test]
fn the_message_lookup_goes_through_the_spy_gate() {
    let g = open();
    let before = g.store.calls.get();
    g.store.message_room("anything").unwrap();
    assert_eq!(g.store.calls.get(), before + 1);
    g.store.fail.set(true);
    assert!(g.store.message_room("anything").is_err());
}

#[test]
fn a_message_ref_to_a_message_in_the_asks_own_room_is_accepted() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    let own = g.post(&alice(), "r", "earlier").unwrap().id;
    ask(&g, &[r("message", &own)]).unwrap();
}

#[test]
fn a_message_ref_into_the_asks_own_room_is_not_judged_against_the_ask_label() {
    let g = open();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.join(&alice(), "vault").unwrap();
    let own = g.post(&alice(), "vault", "earlier").unwrap().id;
    // the room's own label is the room's business, a ref to its own room adds nothing to it
    let asked = g.post_ask(
        &alice(),
        ("vault", "seeking: x", Some("public")),
        None,
        &[r("message", &own), r("room", "vault")],
    );
    assert!(asked.is_ok(), "{asked:?}");
}
