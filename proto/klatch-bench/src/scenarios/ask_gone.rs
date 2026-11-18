use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "a_helper_waiting_on_an_asker_that_never_said_done_is_told_it_is_gone",
            origin: "#226, DESIGN item 89",
            layer: "functional",
            run: helper_told_asker_is_gone,
        },
        Scenario {
            name: "a_helper_is_told_in_seconds_that_its_askers_process_exited",
            origin: "#790, DESIGN item 94",
            layer: "fault",
            run: helper_told_asker_process_exited,
        },
    ]
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

/// #790: the asker is a real fake session (a process named `claude`, a registry record). While it lives a helper's
/// wait says nothing; once its process is gone, with the asker seen only seconds ago, the helper's wait ends within
/// seconds saying so, long before the ten minutes the board takes. A session that said done after an answer is
/// not named.
fn helper_told_asker_process_exited(w: &World) -> Res {
    w.set_clock(T0);
    let h = FakeHarness::new(w)?;
    let asker = h.start("aaaa-0001", "asker")?;
    let helper = h.start("bbbb-0002", "helper")?;
    asker.hook("SessionStart")?;
    helper.hook("SessionStart")?;
    asker.at(T0)?;
    helper.at(T0)?;
    asker.set_turn("busy", T0)?;
    let (ok, out) = asker.klatch(&["post", "lobby", "seeking: a thing"])?;
    check(ok, format!("the asker asks: {out}"))?;
    let (ok, out) = helper.klatch(&["post", "lobby", "answer to #1: here"])?;
    check(ok, format!("the helper answers: {out}"))?;
    let (ok, out) = helper.klatch(&["wait", "lobby", "--timeout", "0"])?;
    check(
        ok && !out.contains("has exited"),
        format!("a live asker is not named: {out}"),
    )?;
    asker.die()?;
    helper.at(T0 + 5)?;
    let (ok, out) = helper.klatch(&["wait", "lobby", "--timeout", "0"])?;
    check(
        ok && out.contains("asker has exited (its process is gone; it never said done)"),
        format!("five seconds after the process ended the helper is told: {out}"),
    )?;
    let started = Instant::now();
    let (ok, out) = helper.klatch(&["wait", "lobby", "--timeout", "120"])?;
    check(
        ok && out.contains("[klatch wait: asker-exited room=lobby") && out.contains("has exited"),
        format!("a long wait ends on the exit with its own marker: {out}"),
    )?;
    check(
        started.elapsed().as_secs() < 20,
        format!(
            "and it ends in seconds, not at the timeout: {:?}",
            started.elapsed()
        ),
    )?;
    Ok(vec![])
}
