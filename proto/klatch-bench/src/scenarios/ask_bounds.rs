use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "an_asker_whose_ask_stays_unanswered_is_told_to_stop_waiting_and_tell_its_user",
        origin: "#200, DESIGN item 88",
        layer: "functional",
        run: asker_told_to_stop_waiting,
    }]
}

/// #200: an asker that waits on its own ask is told at five minutes to stop waiting and say what it is blocked on;
/// a closed ask says nothing.
fn asker_told_to_stop_waiting(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("alice", &["join", "lobby"])?;
    w.ok("bob", &["join", "lobby"])?;
    w.ok("alice", &["post", "lobby", "seeking: a rare thing"])?;
    w.advance(299);
    lacks(
        &w.ok("alice", &["wait", "lobby", "--timeout", "0"])?,
        "Stop waiting",
        "not before the bound",
    )?;
    w.advance(1);
    let told = w.ok("alice", &["wait", "lobby", "--timeout", "0"])?;
    has(
        &told,
        "[klatch wait: timeout room=lobby",
        "the marker stays",
    )?;
    has(
        &told,
        "your ask #1 has had no answer for 5 minutes",
        "told at five minutes",
    )?;
    has(
        &told,
        "say to your user what you are blocked on",
        "the stated message",
    )?;
    w.ok("alice", &["post", "lobby", "closed #1"])?;
    lacks(
        &w.ok("alice", &["wait", "lobby", "--timeout", "0"])?,
        "Stop waiting",
        "a closed ask is not named",
    )?;
    Ok(vec![])
}
