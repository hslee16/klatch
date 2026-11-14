use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "ask_claim_answer_close",
            origin: "run 13, #225",
            layer: "functional",
            run: ask_claim_answer_close,
        },
        Scenario {
            name: "a_release_does_not_end_an_ask",
            origin: "#225",
            layer: "functional",
            run: release_does_not_end_an_ask,
        },
        Scenario {
            name: "decline_busy_versus_cannot",
            origin: "#230",
            layer: "functional",
            run: decline_busy_versus_cannot,
        },
        Scenario {
            name: "an_ask_claim_lapses_and_the_ask_is_unclaimed_again",
            origin: "#233, #292",
            layer: "fault",
            run: claim_lapse_unclaims,
        },
        Scenario {
            name: "a_stale_ask_is_reported_to_its_asker_after_a_minute",
            origin: "#225, #292",
            layer: "functional",
            run: stale_ask_by_clock,
        },
        Scenario {
            name: "a_short_ask_lease_lapses_and_the_asker_is_told",
            origin: "#233",
            layer: "functional",
            run: lease_lapse_told,
        },
        Scenario {
            name: "a_question_waiting_on_the_asker_pauses_the_stale_report",
            origin: "#231",
            layer: "functional",
            run: need_info_pauses_stale,
        },
        Scenario {
            name: "a_working_room_shows_on_the_ask_and_keeps_it_from_going_stale",
            origin: "#231",
            layer: "functional",
            run: working_room,
        },
        Scenario {
            name: "partial_work_is_handed_to_the_next_helper",
            origin: "#233",
            layer: "functional",
            run: partial_handoff,
        },
    ]
}

fn ask_claim_answer_close(w: &World) -> Res {
    w.ok("alice", &["join", "lobby"])?;
    w.ok("alice", &["post", "lobby", "seeking: a thing"])?;
    w.ok("bob", &["claim", "lobby", "ask:1"])?;
    let c = w.refused("carol", &["claim", "lobby", "ask:1"])?;
    has(&c, "bob", "the loser's refusal names the winner")?;
    has(
        &w.ok("carol", &["asks", "lobby"])?,
        "claimed",
        "the ask shows as claimed",
    )?;
    w.ok("bob", &["post", "lobby", "answer to #1: here"])?;
    w.ok("bob", &["release", "lobby", "ask:1"])?;
    lacks(
        &w.ok("carol", &["asks", "lobby"])?,
        "#1",
        "an answered ask is still open",
    )?;
    w.ok("alice", &["post", "lobby", "closed #1"])?;
    Ok(vec![])
}

fn release_does_not_end_an_ask(w: &World) -> Res {
    w.ok("alice", &["post", "lobby", "seeking: a thing"])?;
    w.ok("bob", &["claim", "lobby", "ask:1"])?;
    w.ok("bob", &["release", "lobby", "ask:1"])?;
    let asks = w.ok("carol", &["asks", "lobby"])?;
    has(&asks, "#1", "the ask must stay open after a release")?;
    has(&asks, "unclaimed", "and be unclaimed again")?;
    Ok(vec![])
}

fn decline_busy_versus_cannot(w: &World) -> Res {
    w.ok("alice", &["join", "lobby"])?;
    w.ok("alice", &["post", "lobby", "seeking: a rare thing"])?;
    w.ok("h1", &["post", "lobby", "declined #1 cannot: not mine"])?;
    lacks(
        &w.ok("alice", &["hook"])?,
        "nobody here may be able",
        "one cannot is not enough",
    )?;
    w.ok("h2", &["post", "lobby", "declined #1 cannot: no"])?;
    let told = w.ok("alice", &["hook"])?;
    has(&told, "nobody here may be able", "two cannots escalate")?;
    lacks(
        &w.ok("alice", &["hook"])?,
        "nobody here may be able",
        "told once",
    )?;
    // A busy helper means someone capable exists: no escalation for a second ask.
    w.ok("alice", &["post", "lobby", "seeking: another thing"])?;
    w.ok("h1", &["post", "lobby", "declined #4 cannot: no"])?;
    w.ok("h2", &["post", "lobby", "declined #4 cannot: no"])?;
    w.ok("h3", &["post", "lobby", "declined #4 busy: later"])?;
    lacks(
        &w.ok("alice", &["hook"])?,
        "your ask #4: nobody",
        "a busy helper prevents the escalation",
    )?;
    Ok(vec![])
}

fn claim_lapse_unclaims(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("alice", &["join", "lobby"])?;
    w.ok("alice", &["post", "lobby", "seeking: a thing"])?;
    w.ok("bob", &["claim", "lobby", "ask:1", "--ttl", "60"])?;
    w.advance(30);
    has(
        &w.ok("carol", &["asks", "lobby"])?,
        " claimed ",
        "claimed while the lease lasts",
    )?;
    w.advance(31);
    let asks = w.ok("carol", &["asks", "lobby"])?;
    has(&asks, "unclaimed", "unclaimed again once the lease lapses")?;
    Ok(vec![])
}

fn stale_ask_by_clock(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("alice", &["join", "lobby"])?;
    w.ok("alice", &["post", "lobby", "seeking: a rare thing"])?;
    w.advance(30);
    lacks(
        &w.ok("alice", &["hook"])?,
        "has had no claim",
        "not stale after 30 s",
    )?;
    w.advance(31);
    let told = w.ok("alice", &["hook"])?;
    has(
        &told,
        "your ask #1 has had no claim for 61s",
        "told after a minute",
    )?;
    w.advance(1);
    lacks(&w.ok("alice", &["hook"])?, "has had no claim", "told once")?;
    Ok(vec![])
}

fn lease_lapse_told(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("asker", &["join", "lobby"])?;
    w.ok("asker", &["post", "lobby", "seeking: a thing"])?;
    has(
        &w.ok("helper", &["claim", "lobby", "ask:1"])?,
        "for 90s",
        "an ask claim defaults to a 90 second lease",
    )?;
    w.advance(80);
    w.ok("helper", &["claim", "lobby", "ask:1"])?; // renewed while working
    w.advance(40);
    has(
        &w.ok("viewer", &["asks", "lobby"])?,
        " claimed ",
        "a renewed lease is still held",
    )?;
    lacks(&w.ok("asker", &["hook"])?, "lapsed", "nothing lapsed yet")?;
    w.advance(60);
    let told = w.ok("asker", &["hook"])?;
    has(
        &told,
        "helper's claim lapsed without an answer; the ask is open again",
        "the lapse is told to the asker",
    )?;
    w.advance(1);
    lacks(&w.ok("asker", &["hook"])?, "lapsed", "once")?;
    Ok(vec![])
}

fn need_info_pauses_stale(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("asker", &["join", "lobby"])?;
    w.ok("asker", &["post", "lobby", "seeking: a thing"])?;
    w.ok("helper", &["post", "lobby", "need-info #1: which format?"])?;
    w.advance(70);
    let waiting = w.ok("asker", &["hook"])?;
    lacks(
        &waiting,
        "has had no claim",
        "an ask waiting on its asker is not stale",
    )?;
    has(
        &w.ok("viewer", &["asks", "lobby"])?,
        "needs info",
        "and says it waits on the asker",
    )?;
    w.ok("asker", &["post", "lobby", "info #1: json"])?;
    w.advance(10);
    has(
        &w.ok("asker", &["hook"])?,
        "has had no claim",
        "once answered, the stale clock counts again",
    )?;
    Ok(vec![])
}

fn working_room(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("asker", &["join", "lobby"])?;
    w.ok("asker", &["post", "lobby", "seeking: a thing"])?;
    w.advance(30);
    w.ok("asker", &["join", "ask-1"])?;
    w.ok("helper", &["join", "ask-1"])?;
    w.ok("helper", &["post", "ask-1", "looking at it"])?;
    w.advance(40);
    lacks(
        &w.ok("asker", &["hook"])?,
        "has had no claim",
        "an ask with a busy working room is not stale",
    )?;
    let asks = w.ok("viewer", &["asks", "lobby"])?;
    has(
        &asks,
        "room ask-1 2 members",
        "the ask shows its working room",
    )?;
    w.advance(200);
    has(
        &w.ok("asker", &["hook"])?,
        "has had no claim",
        "a quiet room no longer counts",
    )?;
    Ok(vec![])
}

fn partial_handoff(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("asker", &["join", "lobby"])?;
    w.ok("asker", &["post", "lobby", "seeking: a thing"])?;
    w.ok("helper", &["claim", "lobby", "ask:1"])?;
    w.ok(
        "helper",
        &[
            "post",
            "lobby",
            "partial #1: parsed the file; tests not written",
        ],
    )?;
    w.ok("helper", &["release", "lobby", "ask:1"])?;
    w.advance(5);
    let start = w.ok("next", &["hook", "--event", "SessionStart"])?;
    has(
        &start,
        "partial by helper: parsed the file; tests not written",
        "the next helper is handed the partial work",
    )?;
    has(
        &w.ok("viewer", &["asks", "lobby"])?,
        "partial 1",
        "and the ask shows it",
    )?;
    Ok(vec![])
}
