use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "a_librarian_answers_questions_and_cannot_post_claim_or_change_state",
            origin: "#52, R30",
            layer: "fault",
            run: librarian,
        },
        Scenario {
            name: "history_looks_back_for_a_member_and_a_librarian_and_moves_no_cursor",
            origin: "#518, DESIGN section 32 item 60",
            layer: "functional",
            run: history,
        },
    ]
}

fn history(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("worker", &["join", "town"])?;
    for i in 1..=30 {
        w.ok("peer", &["post", "town", &format!("note {i:02}")])?;
    }
    let last = w.ok("worker", &["history", "town", "--last", "2"])?;
    has(&last, "note 30", "the newest message is missing")?;
    check(!last.contains("note 28"), "--last 2 showed a third message")?;
    let page = w.ok("worker", &["history", "town", "--since", "5"])?;
    has(
        &page,
        "[5 more messages not shown; run `klatch history town --since 25`]",
        "a forward page does not count what follows",
    )?;
    // the cursor did not move: every message is still unread
    has(
        &w.ok("worker", &["read", "town", "--peek"])?,
        "note 01",
        "history moved the read cursor",
    )?;
    // a librarian may use it as a query
    check(
        w.admin(&["librarian", "lib"], "").ok,
        "granting the role failed",
    )?;
    has(
        &w.ok("lib", &["history", "town", "--last", "1"])?,
        "note 30",
        "a librarian cannot look back",
    )?;
    Ok(vec![])
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
