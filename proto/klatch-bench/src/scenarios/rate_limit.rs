use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "a_member_over_the_human_set_rate_limit_is_refused_in_every_room_and_others_are_not",
        origin: "#187, R45",
        layer: "fault",
        run: rate_limit,
    }]
}

fn rate_limit(w: &World) -> Res {
    w.set_clock(T0);
    for room in ["one", "two", "three"] {
        w.ok("flooder", &["join", room])?;
        w.ok("bystander", &["join", room])?;
    }
    check(
        w.admin(&["rate-limit", "3", "60"], "").ok,
        "setting the limit failed",
    )?;
    for (i, room) in ["one", "two", "three"].iter().enumerate() {
        w.advance(1);
        w.ok("flooder", &["post", room, &format!("message {i}")])?;
    }
    w.advance(1);
    let e = w.refused(
        "flooder",
        &["post", "one", "a fourth in a room already used"],
    )?;
    has(&e, "rate-limit", "the fourth post is refused by the rule")?;
    has(&e, "wait", "the refusal says how long to wait")?;
    w.ok(
        "bystander",
        &["post", "one", "another member is not limited"],
    )?;
    w.advance(60);
    w.ok("flooder", &["post", "two", "the window has passed"])?;
    check(
        w.admin(&["rate-limit", "--off"], "").ok,
        "lifting the limit failed",
    )?;
    Ok(vec![])
}
