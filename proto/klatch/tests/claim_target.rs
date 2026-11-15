//! #610: `claim lobby ticket 602` once claimed only `ticket` (and `ticket #602` still would, because the shell drops
//! the comment), so two workers conflicted on the wrong thing. A claim names ONE target: refuse the extra words and a
//! bare noun, and say the form that works.
mod common;

use common::*;

#[test]
fn a_claim_with_extra_words_or_a_bare_noun_is_refused_and_names_the_form() {
    let h = Home::new();
    h.ok("a", &["join", "lobby"]);
    let extra = h.fail("a", &["claim", "lobby", "ticket", "602"]);
    assert!(extra.contains("ticket:602"), "{extra}");
    let bare = h.fail("a", &["claim", "lobby", "ticket"]);
    assert!(bare.contains("ticket:602"), "{bare}");
    // nothing was claimed by either attempt
    assert!(!h.ok("a", &["claims", "lobby"]).contains("ticket"));
    // the forms it names work, and a quoted path with spaces is one target
    h.ok("a", &["claim", "lobby", "ticket:602"]);
    h.ok("a", &["claim", "lobby", "my dir/a file.md"]);
    let claims = h.ok("a", &["claims", "lobby"]);
    assert!(claims.contains("ticket:602") && claims.contains("my dir/a file.md"));
}

#[test]
fn the_convention_shows_the_ticket_form() {
    let h = Home::new();
    let start = h.ok("fresh", &["hook", "--event", "SessionStart"]);
    assert!(start.contains("ticket:602"), "{start}");
}
