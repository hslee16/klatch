//! End-to-end tests: run the real binary against a throwaway KLATCH_HOME.
mod common;

use common::*;

#[test]
fn a_new_session_joins_the_default_lobby_at_session_start_and_nothing_else_does() {
    let h = Home::new();
    h.ok("old", &["join", "lobby"]);
    h.ok("fresh", &["hook", "--event", "UserPromptSubmit"]);
    h.ok("fresh", &["hook", "--event", "PostToolUse"]);
    assert!(
        !h.ok("old", &["who", "lobby"]).contains("fresh"),
        "joined outside session start"
    );
    h.ok("fresh", &["hook", "--event", "SessionStart"]);
    assert!(
        h.ok("old", &["who", "lobby"]).contains("fresh"),
        "did not join the default lobby"
    );
}

#[test]
fn a_joining_session_is_not_flooded_with_the_lobbys_history_but_sees_the_board() {
    let h = Home::new();
    h.ok("old", &["join", "lobby"]);
    h.ok("old", &["post", "lobby", "old chatter one"]);
    h.ok("old", &["post", "lobby", "seeking: a rare thing"]);
    h.ok("old", &["post", "lobby", "offering: text helpers"]);
    let start = h.ok("fresh", &["hook", "--event", "SessionStart"]);
    assert!(
        !start.contains("old chatter one"),
        "history flooded the new session: {start}"
    );
    assert!(
        start.contains("[board lobby]") && start.contains("seeking: a rare thing"),
        "no board: {start}"
    );
    assert!(start.contains("offering: text helpers"));
    h.ok("old", &["post", "lobby", "fresh news"]);
    let later = h.ok("fresh", &["hook", "--event", "PostToolUse"]);
    assert!(
        later.contains("fresh news") && !later.contains("old chatter one"),
        "got: {later}"
    );
}

#[test]
fn a_session_already_in_the_lobby_keeps_its_unread_messages_at_session_start() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok("bob", &["post", "lobby", "while you were away"]);
    let start = h.ok("alice", &["hook", "--event", "SessionStart"]);
    assert!(
        start.contains("while you were away"),
        "a resumed session lost its unread: {start}"
    );
}

#[test]
fn no_join_leaves_a_session_out_of_the_lobby() {
    let h = Home::new();
    h.ok("old", &["join", "lobby"]);
    h.ok("fresh", &["hook", "--event", "SessionStart", "--no-join"]);
    assert!(!h.ok("old", &["who", "lobby"]).contains("fresh"));
}

#[test]
fn init_writes_no_join_only_when_asked_and_asks_about_the_lobby() {
    let s = Setup::new();
    s.init(&["--scope", "project", "--yes"], "");
    assert!(
        !hook_cmds(&Setup::read(&s.project_settings()), "SessionStart")[0].contains("--no-join")
    );
    let n = Setup::new();
    n.init(&["--scope", "project", "--yes", "--no-lobby"], "");
    assert!(
        hook_cmds(&Setup::read(&n.project_settings()), "SessionStart")[0].ends_with("--no-join")
    );
    let i = Setup::new();
    let o = i.init(&["--scope", "project", "--board", "elsewhere"], "n\ny\ny\n");
    assert!(
        out(&o).contains("lobby automatically"),
        "no lobby question: {}",
        out(&o)
    );
    assert!(
        hook_cmds(&Setup::read(&i.project_settings()), "SessionStart")[0].ends_with("--no-join")
    );
}

#[test]
fn a_session_start_that_joins_shows_the_convention_once_with_the_binary_path() {
    let h = Home::new();
    let start = h.ok("fresh", &["hook", "--event", "SessionStart"]);
    assert!(
        start.contains("[klatch convention v1]"),
        "no convention: {start}"
    );
    assert!(
        start.contains(env!("CARGO_BIN_EXE_klatch")),
        "no binary path: {start}"
    );
    assert_eq!(start.matches("[klatch convention v1]").count(), 1);
    assert!(start.contains("seeking:") && start.contains("offering:"));
    let again = h.ok("fresh", &["hook", "--event", "SessionStart"]);
    assert!(
        again.contains("[klatch convention v1]"),
        "a restarted session needs it too: {again}"
    );
}

#[test]
fn the_convention_is_not_shown_outside_session_start_or_to_a_session_that_stays_out() {
    let h = Home::new();
    assert!(
        !h.ok("a", &["hook", "--event", "UserPromptSubmit"])
            .contains("convention")
    );
    assert!(
        !h.ok("a", &["hook", "--event", "PostToolUse"])
            .contains("convention")
    );
    let out = h.ok("b", &["hook", "--event", "SessionStart", "--no-join"]);
    assert!(
        !out.contains("convention"),
        "shown to a session that stays out: {out}"
    );
}

#[test]
fn the_claim_that_closes_a_cycle_is_refused_as_a_deadlock_naming_who_yields() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.ok("alice", &["claim", "r", "X.py"]);
    h.ok("bob", &["claim", "r", "Y.py"]);
    let first = h.fail("alice", &["claim", "r", "Y.py"]);
    assert!(!first.contains("deadlock"), "no cycle yet: {first}");
    let second = h.fail("bob", &["claim", "r", "X.py"]);
    assert!(
        second.contains("deadlock") && second.contains("you yield"),
        "got: {second}"
    );
    assert!(
        second.contains("alice"),
        "does not name the holder: {second}"
    );
    h.ok("bob", &["release", "r", "Y.py"]);
    h.ok("alice", &["claim", "r", "Y.py"]);
}

#[test]
fn a_release_note_reaches_a_member_who_saw_the_claim_and_the_next_claimant() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok("bob", &["claim", "lobby", "src/a.rs"]);
    let seen = h.ok("alice", &["hook", "--event", "PostToolUse"]);
    assert!(seen.contains("src/a.rs"), "{seen}");
    h.ok(
        "bob",
        &["release", "lobby", "src/a.rs", "--note", "rewrote parse()"],
    );
    let told = h.ok("alice", &["hook", "--event", "PostToolUse"]);
    assert!(
        told.contains("no longer holds src/a.rs: rewrote parse() (re-read it before you edit)"),
        "{told}"
    );
    let claimed = h.ok("carol", &["claim", "lobby", "src/a.rs"]);
    assert!(
        claimed.contains("last released by bob") && claimed.contains("rewrote parse()"),
        "{claimed}"
    );
    // a claim on a path nobody released with a note says nothing extra
    let plain = h.ok("carol", &["claim", "lobby", "other.rs"]);
    assert!(!plain.contains("last released"), "{plain}");
}

#[test]
fn admin_commands_refuse_without_a_real_terminal_and_change_nothing() {
    let h = Home::new();
    h.ok("alice", &["post", "lobby", "hello"]);
    for args in [
        vec!["admin", "pause"],
        vec!["admin", "close", "lobby"],
        vec!["admin", "remove", "alice", "--room", "lobby"],
        vec!["admin", "resume"],
    ] {
        let err = h.fail("alice", &args);
        // run by an agent (as here) it is refused for the session above it; run by a script, for having no terminal
        assert!(
            err.contains("real terminal") || err.contains("coding-agent session"),
            "{args:?}: {err}"
        );
    }
    h.ok("alice", &["post", "lobby", "still works"]);
    assert!(
        h.fail("alice", &["admin", "remove", "alice"])
            .contains("--room")
    );
}
