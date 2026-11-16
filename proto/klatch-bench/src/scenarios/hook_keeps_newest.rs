use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "a_busy_room_hook_keeps_the_newest_and_the_addressed_and_counts_what_it_left_out",
        origin: "#185, R44",
        layer: "fault",
        run: busy_room,
    }]
}

fn busy_room(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("listener", &["join", "lobby"])?;
    w.ok("chatty", &["join", "lobby"])?;
    w.ok(
        "chatty",
        &[
            "post",
            "lobby",
            "@listener the very first message is for you",
        ],
    )?;
    for i in 1..=30 {
        w.advance(1);
        w.ok("chatty", &["post", "lobby", &format!("filler number {i}")])?;
    }
    let seen = w.ok("listener", &["hook", "--event", "UserPromptSubmit"])?;
    has(
        &seen,
        "the very first message is for you",
        "the addressed message is kept",
    )?;
    has(&seen, "filler number 30", "the newest is kept")?;
    lacks(&seen, "filler number 1\n", "an old filler is left out")?;
    has(
        &seen,
        "more messages not shown",
        "what was left out is counted",
    )?;
    has(
        &seen,
        "klatch read lobby --peek",
        "and the way to read it is named",
    )?;
    // the cap leaves the messages unread
    let read = w.ok("listener", &["read", "lobby", "--peek"])?;
    has(&read, "filler number 1", "nothing was lost")?;
    Ok(vec![])
}
