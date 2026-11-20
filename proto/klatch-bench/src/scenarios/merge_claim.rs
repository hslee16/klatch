use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "the_merge_main_claim_is_taken_by_an_integrator_and_lapses_when_its_holder_goes_silent",
        origin: "#865, #863, #533",
        layer: "functional",
        run: merge_claim,
    }]
}

fn merge_claim(w: &World) -> Res {
    w.set_clock(T0);
    for who in ["ivy", "ian", "bob"] {
        w.ok(who, &["join", "lobby"])?;
    }
    for who in ["ivy", "ian"] {
        check(
            w.admin(&["integrator", who], "").ok,
            "granting the role failed",
        )?;
    }
    // a plain member is refused with the rule and the next step
    w.advance(5);
    let t = Instant::now();
    let said = w.refused("bob", &["claim", "lobby", "merge:main"])?;
    let refuse_ms = ms(t);
    has(&said, "integrator", "the rule is named")?;
    has(&said, "klatch admin integrator", "the next step is named")?;
    // an integrator takes it and a second one is told who holds it
    let t = Instant::now();
    w.ok("ivy", &["claim", "lobby", "merge:main"])?;
    let take_ms = ms(t);
    has(
        &w.refused("ian", &["claim", "lobby", "merge:main"])?,
        "ivy",
        "the holder is named",
    )?;
    // the holder waits for an event: a park stretches the claim only to the bound, not to the return time
    w.advance(100);
    w.ok(
        "ivy",
        &[
            "post",
            "lobby",
            "parked: waiting on a check suite, back by 3h",
        ],
    )?;
    w.advance(1799);
    w.refused("ian", &["claim", "lobby", "merge:main"])?;
    // it goes silent (the forwarder died): the claim lapses at the bound and the next integrator takes it
    w.advance(2);
    w.ok("ian", &["claim", "lobby", "merge:main"])?;
    has(
        &w.ok("bob", &["claims", "lobby"])?,
        "ian",
        "the new holder is listed",
    )?;
    Ok(vec![
        ("refuse_ms".into(), refuse_ms),
        ("take_ms".into(), take_ms),
        ("park_bound_secs".into(), 1800.0),
    ])
}
