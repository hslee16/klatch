use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "roles_act_as_themselves_through_a_member_path_that_never_reaches_a_floor",
        origin: "#506, #507, #509",
        layer: "fault",
        run: roles_path,
    }]
}

fn roles_path(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("worker", &["join", "town"])?;
    check(
        w.admin(&["admin-agent", "ada"], "").ok,
        "granting admin agent failed",
    )?;
    check(
        w.admin(&["reviewer", "rev"], "").ok,
        "granting reviewer failed",
    )?;
    // an ordinary member is refused the act, and the room stays open
    w.advance(600);
    has(
        &w.refused("worker", &["act", "close", "town"])?,
        "admin agent",
        "member act",
    )?;
    w.ok("worker", &["post", "town", "still open"])?;
    // the admin agent calls its acts as itself, and they are audited with its name
    w.ok("ada", &["act", "close", "town"])?;
    has(
        &w.refused("worker", &["post", "town", "closed now"])?,
        "closed",
        "post to a closed room",
    )?;
    w.ok("ada", &["act", "reopen", "town"])?;
    has(&w.ok("ada", &["act", "audit"])?, "close", "audit")?;
    // no floor act has a member-side form, and the terminal-gated command stays the human's
    for floor in ["cap", "chain", "scrub-add"] {
        check(
            !w.run("ada", &["act", floor, "internal"]).ok,
            format!("{floor} must not be an act"),
        )?;
    }
    check(
        !w.run("ada", &["admin", "cap", "internal"]).ok,
        "klatch admin must stay human-only",
    )?;
    // the reviewer reviews through its own commands; nobody else may
    has(
        &w.ok("rev", &["review", "next"])?,
        "no review",
        "empty queue",
    )?;
    w.advance(600);
    has(
        &w.refused("worker", &["review", "next"])?,
        "not a reviewer",
        "member review",
    )?;
    w.advance(600);
    has(
        &w.refused("rev", &["act", "close", "town"])?,
        "reviewer",
        "reviewer act",
    )?;
    Ok(vec![])
}
