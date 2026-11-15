use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "a_librarian_answers_questions_and_cannot_post_claim_or_change_state",
        origin: "#52, R30",
        layer: "fault",
        run: librarian,
    }]
}

fn librarian(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("worker", &["join", "town"])?;
    w.ok("worker", &["post", "town", "the build is green"])?;
    check(
        w.admin(&["librarian", "lib"], "").ok,
        "granting the role failed",
    )?;
    // it can look without moving anything
    has(
        &w.ok("lib", &["read", "town", "--peek"])?,
        "the build is green",
        "a librarian reads a room",
    )?;
    // and nothing else; time moves so the breaker stays out of it
    for (what, args) in [
        ("post", vec!["post", "town", "hello"]),
        ("claim", vec!["claim", "town", "src/lib.rs"]),
        ("status", vec!["status", "town", "done"]),
    ] {
        w.advance(600);
        has(&w.refused("lib", &args)?, "librarian", what)?;
    }
    // a human can take the role away again
    check(
        w.admin(&["librarian", "lib", "--off"], "").ok,
        "removing the role failed",
    )?;
    w.ok("lib", &["post", "town", "ordinary again"])?;
    Ok(vec![])
}
