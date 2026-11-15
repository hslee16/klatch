use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "the_null_judge_drains_nothing_and_a_resident_judge_is_refused",
        origin: "#48, D4, R31",
        layer: "fault",
        run: null_judge,
    }]
}

fn null_judge(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("alice", &["post", "r", "hello"])?;
    // no judge is configured: one pass decides nothing and every function still works
    let once = w.run("launcher", &["judge", "run", "--once"]);
    check(once.ok, format!("judge run --once failed: {}", once.stderr))?;
    has(&once.stdout, "judged 0 item(s)", "wrong summary")?;
    has(&once.stdout, "0 left queued", "wrong queue count")?;
    has(&w.ok("bob", &["read", "r"])?, "hello", "reading broke")?;
    // there is no daemon: a bare `judge run` says so instead of looping
    let bare = w.run("launcher", &["judge", "run"]);
    check(!bare.ok, "judge run without --once was accepted")?;
    has(&bare.stderr, "--once", "the refusal does not say why")?;
    Ok(vec![])
}
