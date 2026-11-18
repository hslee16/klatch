use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "two_members_each_holding_the_others_ask_are_told_who_yields_and_the_cycle_ends_when_it_does",
        origin: "#273, DESIGN item 90",
        layer: "functional",
        run: ask_cycle_told_and_ended,
    }]
}

/// #273: alice asks, bob asks, each claims the other's ask, so each waits on the other. Each is told when it is about
/// to wait, the first name yields, and `asks` shows the cycle until the yielder lets go of the ask it holds.
fn ask_cycle_told_and_ended(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("alice", &["join", "lobby"])?;
    w.ok("bob", &["join", "lobby"])?;
    w.ok("carol", &["join", "lobby"])?;
    w.ok("alice", &["post", "lobby", "seeking: the schema"])?;
    w.ok("bob", &["post", "lobby", "seeking: the client"])?;
    w.ok("bob", &["claim", "lobby", "ask:1"])?;
    lacks(
        &w.ok("alice", &["wait", "lobby", "--timeout", "0"])?,
        "ask cycle",
        "one claim is a helper working, not a cycle",
    )?;
    w.ok("alice", &["claim", "lobby", "ask:2"])?;
    let told = w.ok("alice", &["wait", "lobby", "--timeout", "0"])?;
    has(
        &told,
        "ask cycle: alice waits on bob (ask #1), bob waits on alice (ask #2)",
        "the cycle is named",
    )?;
    has(&told, "You yield", "the first name is told to yield")?;
    has(&told, "release ask:2", "and what to let go of")?;
    let other = w.ok("bob", &["wait", "lobby", "--timeout", "0", "--peek"])?;
    has(&other, "alice yields", "the other is told who yields")?;
    has(
        &w.ok("carol", &["asks", "lobby"])?,
        "ask cycle: alice waits on bob",
        "anyone can see it in asks",
    )?;
    w.ok("alice", &["release", "lobby", "ask:2"])?;
    lacks(
        &w.ok("carol", &["asks", "lobby"])?,
        "ask cycle",
        "the cycle ends when the yielder lets go",
    )?;
    Ok(vec![])
}
