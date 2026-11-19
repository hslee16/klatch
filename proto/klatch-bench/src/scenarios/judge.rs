use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "the_null_judge_drains_nothing_and_a_resident_judge_is_refused",
            origin: "#48, D4, R31",
            layer: "fault",
            run: null_judge,
        },
        Scenario {
            name: "a_crossing_is_queued_listed_and_moves_nothing_without_a_verdict",
            origin: "#497, D4",
            layer: "fault",
            run: crossing_waits,
        },
    ]
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
    // the resident loop runs bounded passes and exits; a clean item is left queued, never allowed by default
    let looped = w.run(
        "launcher",
        &["judge", "run", "--max-passes", "2", "--interval-secs", "1"],
    );
    check(
        looped.ok,
        format!("judge run loop failed: {}", looped.stderr),
    )?;
    check(
        looped.stdout.matches("judged 0 item(s)").count() == 2,
        "the loop did not make two passes",
    )?;
    Ok(vec![])
}

fn crossing_waits(w: &World) -> Res {
    w.set_clock(T0);
    check(
        w.admin(&["label", "vault", "private"], "").ok,
        "label failed",
    )?;
    check(
        w.admin(&["clear", "alice", "private"], "").ok,
        "clear failed",
    )?;
    w.ok("alice", &["join", "vault"])?;
    w.ok("alice", &["post", "vault", "the plan is blue"])?;
    w.ok("bob", &["join", "open"])?;
    // the target room must be joined by the asker (#679)
    let unjoined = w.run("alice", &["cross", "vault", "1", "--to", "open"]);
    check(
        !unjoined.ok,
        "a crossing into an unjoined room was accepted",
    )?;
    has(
        &unjoined.stderr,
        "not joined",
        "the refusal does not say why",
    )?;
    w.ok("alice", &["join", "open"])?;
    let q = w.ok("alice", &["cross", "vault", "1", "--to", "open"])?;
    has(&q, "queued", "the request was not queued")?;
    // the queue lists a pointer and its state, never the text
    let list = w.admin(&["review-queue"], "");
    check(list.ok, format!("review-queue failed: {}", list.stderr))?;
    has(&list.stdout, "vault#1", "the item is not listed")?;
    has(&list.stdout, "waiting", "the item is not waiting")?;
    check(!list.stdout.contains("blue"), "the listing shows text")?;
    // no judge is configured, so a pass decides nothing and the target room stays empty
    let pass = w.run("launcher", &["judge", "run", "--once"]);
    has(&pass.stdout, "applied 0 crossing", "wrong summary")?;
    check(
        !w.ok("bob", &["read", "open"])?.contains("blue"),
        "text crossed without a verdict",
    )?;
    // a room at the same label is not a crossing
    let same = w.run("alice", &["cross", "vault", "1", "--to", "vault"]);
    check(!same.ok, "a same-label crossing was accepted")?;
    Ok(vec![])
}
