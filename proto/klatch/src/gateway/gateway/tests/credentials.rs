use super::*;

const TOKEN: &str = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";

fn refusal(e: Error) -> (String, String) {
    match e {
        Error::Refused { rule, reason } => (rule, reason),
        other => panic!("not a refusal: {other}"),
    }
}

#[test]
fn a_post_carrying_a_credential_is_refused_by_kind_without_echoing_it() {
    let g = open();
    let err = g
        .post(&alice(), "r", &format!("use this: {TOKEN}"))
        .unwrap_err();
    let (rule, reason) = refusal(err);
    assert_eq!(rule, "credentials");
    assert!(reason.contains("a GitHub token"), "{reason}");
    assert!(
        !reason.contains(TOKEN) && !reason.contains("abcdefgh"),
        "{reason}"
    );
    assert_eq!(g.store.calls.get(), 0, "nothing reached the store");
}

#[test]
fn a_credential_is_refused_in_a_claim_reason_a_release_note_and_a_status() {
    let g = open();
    let who = alice();
    let refused = [
        (
            "claim reason",
            is_refused(g.claim(&who, "r", &ClaimSpec::new("p", 10, TOKEN))),
        ),
        (
            "release note",
            is_refused(g.release_with_note(&who, ("r", "p"), TOKEN)),
        ),
        ("status", is_refused(g.set_status(&who, "r", TOKEN))),
    ];
    all_true(&refused, "carrying a credential must be refused");
    assert_eq!(g.store.calls.get(), 0);
}

#[test]
fn ordinary_messages_still_post() {
    let g = open();
    g.post(
        &alice(),
        "r",
        "seeking: a helper, see commit 3f9c1d2a8b7e6f5d4c3b2a19",
    )
    .unwrap();
}
