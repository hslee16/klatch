//! `klatch ask` and `klatch answer` (#241): the whole ask and the whole answer in one call each, through the same
//! gateway operations as `post`, `claim` and `release`.
mod common;

use common::*;

fn room_with_two(h: &Home) {
    h.ok("alice", &["join", "lobby"]);
    h.ok("bob", &["join", "lobby"]);
}

fn number_in(out: &str) -> String {
    out.split('#')
        .nth(1)
        .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
        .unwrap_or_else(|| panic!("no #N in {out}"))
        .to_string()
}

#[test]
fn ask_posts_a_seeking_message_and_reports_its_number() {
    let h = Home::new();
    room_with_two(&h);
    let said = h.ok("alice", &["ask", "lobby", "who", "knows", "slugify"]);
    let n = number_in(&said);
    let read = h.ok("bob", &["read", "lobby"]);
    assert!(read.contains("seeking: who knows slugify"), "{read}");
    let asks = h.ok("bob", &["asks", "lobby"]);
    assert!(asks.contains(&format!("#{n}")), "{asks}");
}

#[test]
fn ask_does_not_double_the_prefix_and_refuses_empty_text() {
    let h = Home::new();
    room_with_two(&h);
    h.ok("alice", &["ask", "lobby", "seeking: a thing"]);
    let read = h.ok("bob", &["read", "lobby"]);
    assert!(read.contains("seeking: a thing"), "{read}");
    assert!(!read.contains("seeking: seeking:"), "{read}");
    assert!(h.fail("alice", &["ask", "lobby"]).contains("text"));
}

#[test]
fn answer_claims_posts_and_releases_in_one_call() {
    let h = Home::new();
    room_with_two(&h);
    let n = number_in(&h.ok("alice", &["ask", "lobby", "slugify?"]));
    let said = h.ok(
        "bob",
        &["answer", "lobby", &n, "use", "the", "slug", "crate"],
    );
    assert!(said.contains(&format!("answered #{n}")), "{said}");
    let read = h.ok("alice", &["read", "lobby"]);
    assert!(
        read.contains(&format!("answer to #{n}: use the slug crate")),
        "{read}"
    );
    let claims = h.ok("alice", &["claims", "lobby"]);
    assert!(!claims.contains(&format!("ask:{n}")), "{claims}");
    let asks = h.ok("alice", &["asks", "lobby"]);
    assert!(!asks.contains("slugify?"), "{asks}");
}

#[test]
fn a_lost_race_is_a_clear_result_and_posts_nothing() {
    let h = Home::new();
    room_with_two(&h);
    h.ok("carol", &["join", "lobby"]);
    let n = number_in(&h.ok("alice", &["ask", "lobby", "slugify?"]));
    h.ok("bob", &["claim", "lobby", &format!("ask:{n}")]);
    let o = h.run("carol", &["answer", "lobby", &n, "me", "first"]);
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(o.status.success(), "{out}");
    assert!(
        out.contains(&format!("not answered: #{n} is claimed by bob")),
        "{out}"
    );
    let read = h.ok("alice", &["read", "lobby"]);
    assert!(!read.contains("me first"), "{read}");
    // the winner's claim is untouched
    assert!(
        h.ok("alice", &["claims", "lobby"])
            .contains(&format!("ask:{n}"))
    );
}

#[test]
fn answering_an_ask_that_is_not_open_is_refused_and_posts_nothing() {
    let h = Home::new();
    room_with_two(&h);
    let n = number_in(&h.ok("alice", &["ask", "lobby", "slugify?"]));
    h.ok("bob", &["answer", "lobby", &n, "done"]);
    let err = h.fail("bob", &["answer", "lobby", &n, "again"]);
    assert!(err.contains(&format!("#{n} is not an open ask")), "{err}");
    let err = h.fail("bob", &["answer", "lobby", "999", "x"]);
    assert!(err.contains("#999 is not an open ask"), "{err}");
    let read = h.ok("alice", &["read", "lobby"]);
    assert!(!read.contains("again"), "{read}");
}
