//! Typed handoffs (#123): an optional kind and payload on a message. klatch stores, labels, scrubs, delivers and
//! audits them and never interprets them.

use super::*;
use crate::Handoff;

fn plan() -> Handoff<'static> {
    Handoff {
        kind: "work-order",
        payload: Some("{\"id\":7,\"files\":[\"a.rs\"]}"),
    }
}

#[test]
fn a_handoff_is_stored_and_delivered_with_its_kind_and_payload_and_a_plain_post_has_none() {
    let g = open();
    g.post_handoff(&alice(), "r", "see the work order", None, &plan())
        .unwrap();
    g.post(&alice(), "r", "just words").unwrap();
    let m = g.read(&bob(), "r", false).unwrap();
    assert_eq!(
        (m[0].kind.as_deref(), m[0].payload.as_deref()),
        (Some("work-order"), Some("{\"id\":7,\"files\":[\"a.rs\"]}"))
    );
    assert_eq!(
        (m[1].kind.as_deref(), m[1].payload.as_deref()),
        (None, None)
    );
    // the digest a hook shows carries them too
    g.join(&bob(), "q").unwrap();
}

#[test]
fn a_kind_may_travel_alone_but_a_payload_needs_a_kind_and_both_are_checked() {
    let g = open();
    g.post_handoff(
        &alice(),
        "r",
        "done",
        None,
        &Handoff {
            kind: "result",
            payload: None,
        },
    )
    .unwrap();
    for bad in ["", "has space", "a/b", "new\nline", &"k".repeat(41)] {
        let h = Handoff {
            kind: bad,
            payload: Some("x"),
        };
        assert!(
            is_invalid(g.post_handoff(&alice(), "r", "b", None, &h)),
            "kind {bad:?}"
        );
    }
    for bad in [
        "",
        "has\u{0}nul",
        &"p".repeat(klatch_core::MAX_PAYLOAD_BYTES + 1),
    ] {
        let h = Handoff {
            kind: "plan",
            payload: Some(bad),
        };
        assert!(
            is_invalid(g.post_handoff(&alice(), "r", "b", None, &h)),
            "a payload of {} bytes",
            bad.len()
        );
    }
    let full = "p".repeat(klatch_core::MAX_PAYLOAD_BYTES);
    g.post_handoff(
        &alice(),
        "r",
        "b",
        None,
        &Handoff {
            kind: "plan",
            payload: Some(&full),
        },
    )
    .unwrap();
}

#[test]
fn the_payload_is_held_to_the_same_rules_as_the_body_and_a_refusal_never_repeats_it() {
    let g = open();
    g.admin_add_scrub_term(&admin(), "hosts", "quokka.internal")
        .unwrap();
    for (what, payload) in [
        (
            "a credential",
            "token ghp_abcdefghijklmnopqrstuvwxyz0123456789",
        ),
        ("a private term", "ssh to QUOKKA.internal"),
    ] {
        let h = Handoff {
            kind: "plan",
            payload: Some(payload),
        };
        let e = g
            .post_handoff(&alice(), "r", "harmless body", None, &h)
            .unwrap_err();
        assert!(matches!(e, Error::Refused { .. }), "{what}: {e}");
        let shown = e.to_string().to_lowercase();
        assert!(
            !shown.contains("ghp_abc") && !shown.contains("quokka"),
            "{what}: the refusal repeated the payload: {shown}"
        );
    }
    assert!(
        g.read(&bob(), "r", false).unwrap().is_empty(),
        "a refused handoff was stored"
    );
    let refusals = g.admin_refusals(&admin(), 10).unwrap();
    assert_eq!(refusals.len(), 2);
}

#[test]
fn a_handoff_follows_the_label_rules_of_the_room() {
    let g = open();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.post_handoff(&alice(), "vault", "inside", None, &plan())
        .unwrap();
    assert!(is_refused(g.post_handoff(
        &bob(),
        "vault",
        "outside",
        None,
        &plan()
    )));
    assert!(is_refused(g.post_handoff(
        &alice(),
        "r",
        "too high",
        Some("private"),
        &plan()
    )));
}

#[test]
fn a_sealed_label_seals_the_payload_and_a_reader_without_the_key_sees_only_a_note() {
    let g = open();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.admin_seal_label(&admin(), "private").unwrap();
    g.use_label_keys(
        klatch_keys::LabelKeys::new().with("private", klatch_keys::Key::from_bytes([9; 32])),
    );
    g.post_handoff(&alice(), "vault", "sealed plan", None, &plan())
        .unwrap();
    assert_eq!(
        g.read(&alice(), "vault", false).unwrap().len(),
        0,
        "own messages are not unread"
    );
    let all = g.tail(&alice(), "vault", (0, 10)).unwrap();
    assert_eq!(
        all[0].payload.as_deref(),
        Some("{\"id\":7,\"files\":[\"a.rs\"]}")
    );
    g.use_label_keys(klatch_keys::LabelKeys::new());
    let blind = g.tail(&alice(), "vault", (0, 10)).unwrap();
    assert!(blind[0].payload.as_deref().unwrap().contains("sealed"));
    assert_eq!(
        blind[0].kind.as_deref(),
        Some("work-order"),
        "the kind is not secret"
    );
}
