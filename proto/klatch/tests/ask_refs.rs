//! `klatch ask ROOM TEXT --ref kind:value` (#1318): the ask carries references to artifacts outside klatch. They show
//! in the asks list, the board, `read`, `wait` and `find`; klatch never fetches or resolves one, and refuses a
//! credential, a cap breached and a ref on a dm ask.
mod common;

use common::*;

fn number_in(out: &str) -> String {
    out.split('#')
        .nth(1)
        .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
        .unwrap_or_else(|| panic!("no #N in {out}"))
        .to_string()
}

fn world() -> Home {
    let h = Home::new();
    for m in ["alice", "bob"] {
        h.ok(m, &["join", "lobby"]);
    }
    h
}

#[test]
fn refs_show_in_the_ask_the_asks_list_the_board_and_read_and_a_snippet_in_read() {
    let h = world();
    let snippet = h.0.join("fail.log");
    std::fs::write(&snippet, "test parser::eof ... FAILED\nboom\x1b[31m").unwrap();
    let snippet = snippet.to_str().unwrap();
    let said = h.ok(
        "alice",
        &[
            "ask",
            "lobby",
            "fix",
            "the",
            "parser",
            "--ref",
            "pr:o/r#12",
            "--ref-label",
            "the fix",
            "--ref",
            "log:ci.log",
            "--ref-label",
            "",
            "--ref-snippet-file",
            "",
            "--ref-snippet-file",
            snippet,
            "--ref",
            "figma-frame:login page",
        ],
    );
    assert!(said.contains("3 refs"), "{said}");
    let asks = h.ok("bob", &["asks", "lobby"]);
    assert!(asks.contains("ref pr o/r#12 (the fix)"), "{asks}");
    assert!(asks.contains("ref log ci.log [snippet"), "{asks}");
    assert!(asks.contains("ref figma-frame login page"), "{asks}");
    let board = h.ok("bob", &["board", "lobby"]);
    assert!(board.contains("ref pr o/r#12 (the fix)"), "{board}");
    let read = h.ok("bob", &["read", "lobby", "--peek"]);
    assert!(
        read.contains(
            "seeking: fix the parser [refs pr o/r#12, log ci.log, figma-frame login page]"
        ),
        "{read}"
    );
    assert!(read.contains("    | test parser::eof ... FAILED"), "{read}");
    assert!(
        !read.contains('\x1b'),
        "control characters are escaped: {read:?}"
    );
    assert!(read.contains("\\u{1b}[31m"), "{read:?}");
}

#[test]
fn an_ask_without_refs_shows_none_and_an_unknown_kind_is_accepted() {
    let h = world();
    h.ok("alice", &["ask", "lobby", "plain"]);
    assert!(!h.ok("bob", &["asks", "lobby"]).contains("ref "));
    h.ok(
        "alice",
        &["ask", "lobby", "odd", "--ref", "whatever-kind:a b c"],
    );
    assert!(
        h.ok("bob", &["asks", "lobby"])
            .contains("ref whatever-kind a b c")
    );
}

#[test]
fn refs_are_refused_not_cut_and_nothing_is_posted() {
    let h = world();
    let creds = h.fail(
        "alice",
        &[
            "ask",
            "lobby",
            "x",
            "--ref",
            "url:https://u:pw@example.com/x",
        ],
    );
    assert!(creds.contains("credential"), "{creds}");
    assert!(!creds.contains("pw@"), "{creds}");
    let bad_kind = h.fail("alice", &["ask", "lobby", "x", "--ref", "Bad:thing"]);
    assert!(bad_kind.contains("kind"), "{bad_kind}");
    let no_colon = h.fail("alice", &["ask", "lobby", "x", "--ref", "justtext"]);
    assert!(no_colon.contains("KIND:VALUE"), "{no_colon}");
    let path = h.fail("alice", &["ask", "lobby", "x", "--ref", "path:../secret"]);
    assert!(path.contains("path"), "{path}");
    let mut many = vec!["ask", "lobby", "x"];
    let refs: Vec<String> = (0..33).map(|i| format!("other:v{i}")).collect();
    for r in &refs {
        many.push("--ref");
        many.push(r);
    }
    let over = h.fail("alice", &many);
    assert!(over.contains("32"), "{over}");
    let big = h.0.join("big.log");
    std::fs::write(&big, "s".repeat(9000)).unwrap();
    let too_big = h.fail(
        "alice",
        &[
            "ask",
            "lobby",
            "x",
            "--ref",
            "log:x",
            "--ref-snippet-file",
            big.to_str().unwrap(),
        ],
    );
    assert!(too_big.contains("snippet"), "{too_big}");
    let missing = h.fail(
        "alice",
        &[
            "ask",
            "lobby",
            "x",
            "--ref",
            "log:x",
            "--ref-snippet-file",
            "/no/such/file",
        ],
    );
    assert!(missing.contains("/no/such/file"), "{missing}");
    let stray = h.fail("alice", &["ask", "lobby", "x", "--ref-label", "orphan"]);
    assert!(stray.contains("--ref"), "{stray}");
    assert!(h.ok("bob", &["read", "lobby", "--peek"]).trim().is_empty());
}

#[test]
fn a_dm_ask_carries_no_refs_and_says_so() {
    let h = world();
    let dm = h.fail("alice", &["ask", "--to", "bob", "x", "--ref", "pr:#1"]);
    assert!(dm.contains("--ref") && dm.contains("room ask"), "{dm}");
}

#[test]
fn find_matches_an_answered_ask_by_its_refs() {
    let h = world();
    h.ok(
        "alice",
        &[
            "ask",
            "lobby",
            "why",
            "is",
            "it",
            "red",
            "--ref",
            "ci-run:nightly-4411",
            "--ref",
            "doc:flaky-tests.md",
        ],
    );
    h.ok("bob", &["post", "lobby", "answer to #1: a flaky test"]);
    let shown = h.ok("carol", &["find", "lobby", "nightly-4411"]);
    assert!(shown.contains("#1") && shown.contains("ref"), "{shown}");
    assert!(shown.contains("ci-run nightly-4411"), "{shown}");
    let none = h.run("carol", &["find", "lobby", "unrelated-thing-xyz"]);
    assert!(!String::from_utf8_lossy(&none.stdout).contains("#1"));
}

#[test]
fn could_answer_shows_the_refs_and_the_verdict_is_unchanged() {
    let h = world();
    let n = number_in(&h.ok("alice", &["ask", "lobby", "x", "--ref", "pr:o/r#12"]));
    let out = h.ok("bob", &["could-answer", "lobby", &n]);
    assert!(out.starts_with("yes"), "{out}");
    assert!(out.contains("ref pr o/r#12"), "{out}");
}
