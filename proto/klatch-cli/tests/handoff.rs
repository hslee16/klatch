//! Typed handoffs at the command line (#123): `post --kind K --payload P`, shown by `read`, marked (never expanded)
//! in the hook, refused by the same rules that refuse a body.
mod common;
use common::*;

#[test]
fn a_handoff_posted_with_a_kind_and_payload_is_read_back_with_both() {
    let h = Home::new();
    h.ok(
        "alice",
        &[
            "post",
            "--kind",
            "work-order",
            "--payload",
            "{\"id\":7}",
            "r",
            "see",
            "the",
            "order",
        ],
    );
    h.ok("alice", &["post", "r", "just words"]);
    let read = h.ok("bob", &["read", "r", "--peek"]);
    assert!(
        read.contains("see the order")
            && read.contains("[handoff work-order")
            && read.contains("{\"id\":7}"),
        "{read}"
    );
    let plain = read.lines().find(|l| l.contains("just words")).unwrap();
    assert!(!plain.contains("handoff"), "{plain}");
}

#[test]
fn the_hook_marks_a_handoff_with_its_kind_and_size_and_never_shows_the_payload() {
    let h = Home::new();
    h.ok("bob", &["join", "r"]);
    h.ok(
        "alice",
        &[
            "post",
            "--kind",
            "plan",
            "--payload",
            "the-payload-text",
            "r",
            "see attached",
        ],
    );
    let hook = h.ok("bob", &["hook"]);
    assert!(
        hook.contains("[handoff plan, 16 bytes of payload"),
        "{hook}"
    );
    assert!(!hook.contains("the-payload-text"), "{hook}");
}

#[test]
fn a_payload_needs_a_kind_and_a_bad_kind_or_payload_is_refused_without_repeating_it() {
    let h = Home::new();
    assert!(
        !h.run("alice", &["post", "--payload", "x", "r", "b"])
            .status
            .success()
    );
    let bad_kind = h.fail(
        "alice",
        &["post", "--kind", "has space", "--payload", "x", "r", "b"],
    );
    assert!(bad_kind.contains("kind"), "{bad_kind}");
    let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let e = h.fail(
        "alice",
        &["post", "--kind", "plan", "--payload", secret, "r", "b"],
    );
    assert!(!e.contains(secret), "the refusal repeated the payload: {e}");
    let big = "p".repeat(20_000);
    let e = h.fail(
        "alice",
        &["post", "--kind", "plan", "--payload", &big, "r", "b"],
    );
    assert!(e.contains("at most"), "{e}");
    assert!(
        h.ok("bob", &["read", "r", "--peek"]).is_empty(),
        "a refused handoff was stored"
    );
}

#[test]
fn wait_shows_a_handoff_too() {
    let h = Home::new();
    h.ok("bob", &["join", "r"]);
    h.ok(
        "alice",
        &[
            "post",
            "--kind",
            "result",
            "--payload",
            "done:ok",
            "r",
            "finished",
        ],
    );
    let waited = h.ok("bob", &["wait", "r", "--timeout", "3", "--peek"]);
    assert!(
        waited.contains("[handoff result") && waited.contains("done:ok"),
        "{waited}"
    );
}
