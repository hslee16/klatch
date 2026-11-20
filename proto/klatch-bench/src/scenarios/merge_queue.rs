use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "ready_words_build_a_merge_queue_on_the_board_that_only_the_integrator_moves",
        origin: "#864, #863, DESIGN s32 item 84",
        layer: "functional",
        run: merge_queue,
    }]
}

fn merge_queue(w: &World) -> Res {
    w.set_clock(T0);
    for who in ["ivy", "alice", "bob"] {
        w.ok(who, &["join", "lobby"])?;
    }
    check(
        w.admin(&["integrator", "ivy"], "").ok,
        "granting the role failed",
    )?;
    lacks(
        &w.ok("bob", &["board", "lobby"])?,
        "queue",
        "no queue section before any ready word",
    )?;
    // two members enqueue; a repeat changes nothing
    w.ok("alice", &["post", "lobby", "ready: #7"])?;
    w.advance(10);
    w.ok("bob", &["post", "lobby", "ready: #8"])?;
    w.ok("bob", &["post", "lobby", "ready: #7"])?;
    let t = Instant::now();
    let board = w.ok("alice", &["board", "lobby"])?;
    let board_ms = ms(t);
    has(&board, "queue (merge:main free):", "the section is shown")?;
    has(&board, "1. #7 waiting (ready by alice", "first in line")?;
    has(&board, "2. #8 waiting (ready by bob", "second in line")?;
    // a plain member cannot move a PR
    w.ok("bob", &["post", "lobby", "integrating: #7"])?;
    has(
        &w.ok("alice", &["board", "lobby"])?,
        "1. #7 waiting",
        "a plain member's state word is ignored",
    )?;
    // the integrator holds the claim and moves #7 through, and drops #8 for a conflict
    w.ok("ivy", &["claim", "lobby", "merge:main"])?;
    w.ok("ivy", &["post", "lobby", "integrating: #7"])?;
    has(
        &w.ok("bob", &["board", "lobby"])?,
        "queue (merge:main held by ivy):\n  1. #7 integrating",
        "the claim holder and the state are shown",
    )?;
    w.ok("ivy", &["post", "lobby", "checking: #7"])?;
    has(
        &w.ok("bob", &["board", "lobby"])?,
        "1. #7 checking",
        "checking",
    )?;
    w.ok("ivy", &["post", "lobby", "merged: #7"])?;
    w.ok("ivy", &["post", "lobby", "dropped: #8 conflict"])?;
    let done = w.ok("bob", &["board", "lobby"])?;
    has(&done, "#7 merged", "merged is shown")?;
    has(
        &done,
        "#8 dropped: code conflict with main",
        "the reason is shown",
    )?;
    // a new ready enqueues the dropped PR again
    w.ok("alice", &["post", "lobby", "ready: #8"])?;
    has(
        &w.ok("alice", &["board", "lobby"])?,
        "1. #8 waiting",
        "a dropped PR can be enqueued again",
    )?;
    Ok(vec![("board_ms".into(), board_ms)])
}
