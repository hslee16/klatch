use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "the_receiver_budget_refuses_floods_and_clips_what_a_session_sees",
            origin: "#22, R22",
            layer: "fault",
            run: receiver_budget,
        },
        Scenario {
            name: "admin_controls_pause_close_remove_and_audit",
            origin: "#23, R11",
            layer: "fault",
            run: admin_controls,
        },
    ]
}

fn receiver_budget(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("reader", &["join", "r"])?;
    let big = "x".repeat(33 * 1024);
    has(
        &w.refused("poster", &["post", "r", &big])?,
        "too large",
        "an oversize post is refused with its reason",
    )?;
    w.ok("poster", &["post", "r", "closed #1"])?;
    has(
        &w.refused("poster", &["post", "r", "closed #1"])?,
        "same as your message",
        "an identical repeat is refused",
    )?;
    for i in 0..100 {
        w.ok("flood", &["post", "r", &format!("spam {i}")])?;
    }
    has(
        &w.refused("flood", &["post", "r", "spam again"])?,
        "too many posts",
        "the 101st post in a minute is refused",
    )?;
    let long = "y".repeat(5000);
    w.ok("poster", &["post", "r", &long])?;
    let seen = w.ok("reader", &["hook", "--event", "UserPromptSubmit"])?;
    check(
        !seen.contains(&"y".repeat(1001)),
        "a long message reached the session unclipped",
    )?;
    has(
        &seen,
        "more messages not shown",
        "the digest counts what it left out",
    )?;
    w.advance(61);
    w.ok("flood", &["post", "r", "after the window"])?;
    Ok(vec![])
}

fn admin_controls(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("alice", &["join", "r"])?;
    w.ok("mallory", &["join", "r"])?;
    w.ok("mallory", &["join", "side"])?;
    w.ok("mallory", &["claim", "r", "a.rs"])?;
    // a wrong confirmation changes nothing
    let r = w.admin(&["pause"], "nope");
    check(!r.ok, "pause went ahead without the confirmation")?;
    w.ok("alice", &["post", "r", "still open"])?;
    // pause: nothing is posted, and a hook says why
    let r = w.admin(&["pause"], "pause");
    check(r.ok, format!("pause failed: {}", r.stderr.trim()))?;
    has(
        &w.refused("alice", &["post", "r", "while paused"])?,
        "paused by the admin",
        "a paused room refuses posts with its reason",
    )?;
    has(
        &w.ok("alice", &["hook", "--event", "UserPromptSubmit"])?,
        "paused by the admin",
        "a hook says why nothing arrives",
    )?;
    check(w.admin(&["resume"], "").ok, "resume failed")?;
    w.ok("alice", &["post", "r", "after resume"])?;
    // close: read-only
    check(w.admin(&["close", "r"], "r").ok, "close failed")?;
    has(
        &w.refused("alice", &["post", "r", "to a closed room"])?,
        "closed by the admin",
        "a closed room refuses posts",
    )?;
    has(
        &w.ok("mallory", &["read", "r", "--peek"])?,
        "after resume",
        "a closed room stays readable",
    )?;
    check(w.admin(&["reopen", "r"], "").ok, "reopen failed")?;
    // remove one member from one room: its claims go, it is refused there and nowhere else
    check(
        w.admin(&["remove", "mallory", "--room", "r"], "mallory").ok,
        "remove failed",
    )?;
    has(
        &w.refused("mallory", &["post", "r", "x"])?,
        "removed from room r",
        "a removed member is refused in that room",
    )?;
    w.ok("mallory", &["post", "side", "other rooms are fine"])?;
    check(
        w.ok("alice", &["claims", "r"])?.trim().is_empty(),
        "the removed member still holds a claim",
    )?;
    check(
        w.admin(&["allow", "mallory", "--room", "r"], "").ok,
        "allow failed",
    )?;
    w.ok("mallory", &["post", "r", "back"])?;
    // the audit shows each act, newest first
    let log = w.admin(&["audit"], "");
    check(log.ok, "audit failed")?;
    let acts: Vec<&str> = log
        .stdout
        .lines()
        .filter_map(|l| l.split('\t').nth(1))
        .collect();
    check(
        acts == ["allow", "remove", "reopen", "close", "resume", "pause"],
        format!("audit order: {acts:?}"),
    )?;
    Ok(vec![])
}
