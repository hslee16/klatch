//! End-to-end tests: run the real binary against a throwaway KLATCH_HOME.
mod common;

use common::*;

#[test]
fn a_member_that_ends_does_not_keep_its_claims_and_the_asker_is_told() {
    let h = Home::new();
    h.ok("asker", &["join", "lobby"]);
    h.ok("asker", &["post", "lobby", "seeking: a thing"]);
    h.ok("helper", &["join", "lobby"]);
    h.ok("helper", &["claim", "lobby", "ask:1"]);
    h.ok("helper", &["claim", "lobby", "src/a.rs"]);
    h.fail("rival", &["claim", "lobby", "src/a.rs"]);
    h.ok("helper", &["hook", "--event", "SessionEnd"]);
    assert!(
        h.ok("rival", &["claims", "lobby"]).trim().is_empty(),
        "the ended member still holds claims"
    );
    h.ok("rival", &["claim", "lobby", "src/a.rs"]);
    let told = h.ok("asker", &["hook", "--event", "UserPromptSubmit"]);
    assert!(
        told.contains("your ask #1: helper's claim lapsed without an answer"),
        "{told}"
    );
}

#[test]
fn session_end_marks_the_member_done_in_every_room_it_joined_and_prints_nothing() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok("alice", &["join", "side"]);
    h.ok("alice", &["post", "lobby", "offering: text helpers"]);
    h.ok("bob", &["join", "lobby"]);
    let out = h.ok("alice", &["hook", "--event", "SessionEnd"]);
    assert_eq!(out, "", "SessionEnd must print nothing: {out}");
    for room in ["lobby", "side"] {
        let who = h.ok("bob", &["who", room]);
        let line = who.lines().find(|l| l.starts_with("alice")).unwrap();
        assert!(line.contains("status: done"), "{room}: {line}");
    }
    assert!(
        h.ok("bob", &["board", "lobby"])
            .contains("no current offers"),
        "the offer stayed on the board"
    );
    assert!(
        h.ok("alice", &["hooks"]).contains("SessionEnd"),
        "the run was not recorded"
    );
}

#[test]
fn session_end_for_a_member_in_no_room_is_harmless() {
    let h = Home::new();
    let o = h.run("loner", &["hook", "--event", "SessionEnd"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

#[test]
fn init_installs_the_session_end_hook_by_default_and_not_with_no_session_end() {
    let s = Setup::new();
    s.init(&["--scope", "project", "--yes"], "");
    let c = hook_cmds(&Setup::read(&s.project_settings()), "SessionEnd");
    assert_eq!(c.len(), 1);
    assert!(
        c[0].ends_with("--discover hook --event SessionEnd"),
        "{}",
        c[0]
    );
    let n = Setup::new();
    n.init(&["--scope", "project", "--yes", "--no-session-end"], "");
    assert!(
        Setup::read(&n.project_settings())["hooks"]
            .get("SessionEnd")
            .is_none()
    );
    let i = Setup::new();
    let o = i.init(&["--scope", "project", "--board", "elsewhere"], "y\nn\ny\n");
    assert!(out(&o).contains("when it ends"), "no question: {}", out(&o));
    assert!(
        Setup::read(&i.project_settings())["hooks"]
            .get("SessionEnd")
            .is_none()
    );
}
