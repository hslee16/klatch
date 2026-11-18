use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "a_helper_waiting_on_an_asker_that_never_said_done_is_told_it_is_gone",
        origin: "#226, DESIGN item 89",
        layer: "functional",
        run: helper_told_asker_is_gone,
    }]
}

/// #226: two askers get answers from one helper. One says done and leaves cleanly; the other crashes (is never
/// seen again and never says done). The helper's wait names only the second, once it has been quiet for the
/// board's ten minutes, and says why.
fn helper_told_asker_is_gone(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("alice", &["join", "lobby"])?;
    w.ok("dana", &["join", "lobby"])?;
    w.ok("alice", &["post", "lobby", "seeking: a thing"])?;
    w.ok("dana", &["post", "lobby", "seeking: another thing"])?;
    w.ok("bob", &["join", "lobby"])?;
    w.ok("bob", &["post", "lobby", "answer to #1: here"])?;
    w.ok("bob", &["post", "lobby", "answer to #2: here too"])?;
    w.ok("bob", &["read", "lobby"])?;
    w.ok("dana", &["status", "lobby", "done"])?;
    w.advance(599);
    lacks(
        &w.ok("bob", &["wait", "lobby", "--timeout", "0"])?,
        "alice has not been seen",
        "not before ten minutes",
    )?;
    w.advance(1);
    let told = w.ok("bob", &["wait", "lobby", "--timeout", "0"])?;
    has(
        &told,
        "[klatch wait: timeout room=lobby",
        "the marker stays",
    )?;
    has(
        &told,
        "alice has not been seen for 10 minutes (it never said done)",
        "the crashed asker is named with why",
    )?;
    has(
        &told,
        "Stop waiting for it",
        "and the helper is told to stop",
    )?;
    lacks(&told, "dana", "an asker that said done left cleanly")?;
    Ok(vec![])
}
