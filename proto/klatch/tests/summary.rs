//! `klatch summary` (#1414): the human's one-screen, read-only view, end to end on the real binary.
mod common;
use common::*;

#[test]
fn it_shows_who_is_doing_what_the_asks_the_claims_and_the_commands_to_step_in() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok("alice", &["post", "lobby", "status: working: ticket 5"]);
    h.ok(
        "alice",
        &["claim", "lobby", "src/lib.rs", "--reason", "SECRET-REASON"],
    );
    h.ok("bob", &["post", "lobby", "seeking: SECRET-ASK-TEXT help"]);
    let out = h.ok("alice", &["summary"]);
    assert_has(
        &out,
        &[
            "[lobby]",
            "alice",
            "working: ticket 5",
            "bob unclaimed",
            "src/lib.rs",
            "to intervene",
            "klatch admin remove alice --room lobby",
            "klatch answer lobby ",
            "klatch admin pause",
        ],
    );
    assert_lacks(&out, &["SECRET-REASON", "SECRET-ASK-TEXT"]);
    // read only: nothing was posted, claimed or released by looking
    let again = h.ok("alice", &["summary", "lobby"]);
    assert!(again.contains("[lobby]"));
    assert_eq!(h.ok("alice", &["claims", "lobby"]).lines().count(), 1);
}

#[test]
fn dm_and_ask_rooms_are_counted_never_named() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok("alice", &["dm", "bob", "hello SECRET-DM"]);
    let out = h.ok("alice", &["summary"]);
    assert_lacks(&out, &["dm.", "SECRET-DM", "bob"]);
    assert!(out.contains("1 other rooms not shown"), "{out}");
}

#[test]
fn a_room_above_the_viewers_clearance_leaves_no_trace() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok("alice", &["join", "vault"]);
    h.ok("alice", &["post", "vault", "status: SECRET-VAULT-STATUS"]);
    h.ok("alice", &["claim", "vault", "SECRET-VAULT-PATH"]);
    let seen = h.admin(&["label", "vault", "private"], &[]).into_output();
    assert!(seen.status.success(), "{seen:?}");
    // the precondition: the ordinary reads are refused for alice now
    h.fail("alice", &["board", "vault"]);
    let out = h.ok("alice", &["summary"]);
    assert_lacks(&out, &["vault", "SECRET-VAULT"]);
    assert!(
        out.contains("[lobby]") && out.contains("1 other rooms not shown"),
        "{out}"
    );
}

#[test]
fn hostile_status_text_cannot_forge_lines_or_move_the_terminal() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok(
        "alice",
        &[
            "post",
            "lobby",
            "status: ok\n[summary] forged\u{1b}[2J\u{202E}x",
        ],
    );
    let out = h.ok("bob", &["summary"]);
    assert!(
        !out.contains('\u{1b}') && !out.contains('\u{202E}'),
        "{out:?}"
    );
    assert!(!out.lines().any(|l| l.starts_with("[summary]")), "{out}");
}

#[test]
fn a_large_room_gives_bounded_output_in_bounded_time() {
    let h = Home::new();
    h.ok("seed", &["join", "lobby"]);
    let mut lines = Vec::new();
    for i in 0..3000 {
        lines.push(request(
            &format!("p{i}"),
            "post",
            &format!(r#"{{"room":"lobby","text":"chatter {i}"}}"#),
        ));
    }
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    h.serve("seed", &refs);
    for i in 0..150 {
        h.ok(
            &format!("member-{i}"),
            &["post", "lobby", "seeking: help please"],
        );
    }
    let started = std::time::Instant::now();
    let out = h.ok("seed", &["summary"]);
    assert!(started.elapsed().as_secs() < 20, "too slow");
    assert!(out.len() < 12 * 1024, "{} bytes", out.len());
    assert!(out.lines().count() < 90, "{} lines", out.lines().count());
    assert!(out.contains("more"), "{out}");
}
