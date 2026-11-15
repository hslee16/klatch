use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "a_quarantined_reviewer_has_no_path_to_a_room",
        origin: "#51, D4",
        layer: "fault",
        run: reviewer,
    }]
}

fn reviewer(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("worker", &["join", "town"])?;
    w.ok("worker", &["post", "town", "the build is green"])?;
    check(
        w.admin(&["reviewer", "rev"], "").ok,
        "granting the role failed",
    )?;
    // it may not look at a room, post, or claim; time moves so the breaker stays out of it
    for (what, args) in [
        ("read", vec!["read", "town", "--peek"]),
        ("post", vec!["post", "town", "hello"]),
        ("claim", vec!["claim", "town", "src/lib.rs"]),
    ] {
        w.advance(600);
        has(&w.refused("rev", &args)?, "reviewer", what)?;
    }
    // a human can take the role away again
    check(
        w.admin(&["reviewer", "rev", "--off"], "").ok,
        "removing the role failed",
    )?;
    w.ok("rev", &["post", "town", "ordinary again"])?;
    Ok(vec![])
}
