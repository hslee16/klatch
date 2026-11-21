//! `klatch ask --to MEMBER`: a private ask as a dm `seeking:` (#1189), answered with `klatch answer dm:MEMBER N`.
mod common;

use common::*;

fn world() -> Home {
    let h = Home::new();
    for m in ["alice", "bob", "carol"] {
        h.ok(m, &["join", "lobby"]);
    }
    h
}

fn number_in(out: &str) -> String {
    out.split('#')
        .nth(1)
        .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
        .unwrap_or_else(|| panic!("no #N in {out}"))
        .to_string()
}

#[test]
fn a_private_ask_opens_the_dm_with_an_open_ask_and_names_the_wait() {
    let h = world();
    let said = h.ok("alice", &["ask", "--to", "bob", "slugify", "how?"]);
    let n = number_in(&said);
    assert!(said.contains("dm.alice.bob"), "{said}");
    assert!(said.contains("`klatch wait dm.alice.bob`"), "{said}");
    let read = h.ok("bob", &["read", "dm.alice.bob"]);
    assert!(read.contains("seeking: slugify how?"), "{read}");
    let asks = h.ok("bob", &["asks", "dm.alice.bob"]);
    assert!(asks.contains(&format!("#{n}")), "{asks}");
}

#[test]
fn the_answer_closes_it_and_the_lobby_hears_nothing() {
    let h = world();
    let n = number_in(&h.ok("alice", &["ask", "--to", "bob", "slugify?"]));
    let said = h.ok(
        "bob",
        &["answer", "dm:alice", &n, "use", "the", "slug", "crate"],
    );
    assert!(said.contains(&format!("answered #{n}")), "{said}");
    let read = h.ok("alice", &["read", "dm.alice.bob"]);
    assert!(
        read.contains(&format!("answer to #{n}: use the slug crate")),
        "{read}"
    );
    assert!(
        !h.ok("alice", &["asks", "dm.alice.bob"])
            .contains("slugify?")
    );
    // the dm room name works too
    let n2 = number_in(&h.ok("alice", &["ask", "--to", "bob", "again?"]));
    h.ok("bob", &["answer", "dm.alice.bob", &n2, "yes"]);
    // nothing reached the lobby: no ask, no pointer, no message
    let lobby = h.ok("carol", &["read", "lobby"]);
    assert!(
        !lobby.contains("slugify") && !lobby.contains("seeking") && !lobby.contains("moved"),
        "{lobby}"
    );
    assert!(!h.ok("carol", &["asks", "lobby"]).contains('#'));
}

#[test]
fn a_third_member_cannot_see_or_answer_it() {
    let h = world();
    let n = number_in(&h.ok("alice", &["ask", "--to", "bob", "private?"]));
    let seen = h.run("carol", &["read", "dm.alice.bob"]);
    assert!(!String::from_utf8_lossy(&seen.stdout).contains("private?"));
    let err = h.fail("carol", &["answer", "dm.alice.bob", &n, "nosy"]);
    assert!(!err.contains("private?"), "{err}");
    let read = h.ok("alice", &["read", "dm.alice.bob"]);
    assert!(!read.contains("nosy"), "{read}");
}

#[test]
fn an_ask_to_an_unknown_member_or_yourself_is_refused_with_a_fixed_message() {
    let h = world();
    let err = h.fail("alice", &["ask", "--to", "zed", "SECRETTEXT"]);
    assert!(
        err.contains("zed is not a member you share a room with"),
        "{err}"
    );
    assert!(!err.contains("SECRETTEXT"), "{err}");
    let err = h.fail("alice", &["ask", "--to", "alice", "me?"]);
    assert!(err.contains("yourself"), "{err}");
    let err = h.fail("alice", &["ask", "--to", "bob"]);
    assert!(err.contains("text"), "{err}");
    // nothing was made
    let err = h.fail("alice", &["wait", "dm:zed", "--timeout", "0"]);
    assert!(err.contains("no direct messages with zed"), "{err}");
}

#[test]
fn ask_help_names_to() {
    let h = world();
    for args in [["help", "ask"], ["ask", "--help"]] {
        let out = h.ok("alice", &args);
        assert!(out.contains("--to"), "{out}");
    }
    let out = h.ok("alice", &["answer", "--help"]);
    assert!(out.contains("dm:"), "{out}");
}
