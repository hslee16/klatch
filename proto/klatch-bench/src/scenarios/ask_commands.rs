//! The one-call ask and answer, the look-back for earlier answers, and the asker's reopen (#241, #239, #232).
use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "ask_and_answer_are_one_call_each_and_a_lost_race_posts_nothing",
            origin: "#241",
            layer: "functional",
            run: ask_answer,
        },
        Scenario {
            name: "find_returns_an_earlier_answered_ask_read_only_and_says_so_when_there_is_none",
            origin: "#239",
            layer: "functional",
            run: find,
        },
        Scenario {
            name: "only_the_asker_can_reopen_an_ask_and_a_later_answer_ends_it_again",
            origin: "#232",
            layer: "functional",
            run: reopen,
        },
    ]
}

fn ask_answer(w: &World) -> Res {
    w.set_clock(T0);
    for who in ["alice", "bob", "carol"] {
        w.ok(who, &["join", "lobby"])?;
    }
    has(
        &w.ok("alice", &["ask", "lobby", "who", "knows", "slugify"])?,
        "asked #1",
        "ask reports its number",
    )?;
    has(
        &w.ok("bob", &["read", "lobby", "--peek"])?,
        "seeking: who knows slugify",
        "the ask carries the seeking prefix",
    )?;
    // the prefix is not doubled
    w.ok("alice", &["ask", "lobby", "seeking: a second thing"])?;
    lacks(
        &w.ok("bob", &["read", "lobby", "--peek"])?,
        "seeking: seeking:",
        "the prefix was doubled",
    )?;
    // a lost race: bob holds ask 2, carol's answer is a clear result and posts nothing
    w.ok("bob", &["claim", "lobby", "ask:2"])?;
    has(
        &w.ok("carol", &["answer", "lobby", "2", "me", "first"])?,
        "not answered: #2 is claimed by bob",
        "the lost race",
    )?;
    lacks(
        &w.ok("alice", &["read", "lobby", "--peek"])?,
        "me first",
        "a lost race posted its answer",
    )?;
    // the one-call answer claims, posts and releases
    has(
        &w.ok(
            "carol",
            &["answer", "lobby", "1", "use", "the", "slug", "crate"],
        )?,
        "answered #1",
        "the answer",
    )?;
    has(
        &w.ok("alice", &["read", "lobby", "--peek"])?,
        "answer to #1: use the slug crate",
        "the answer text",
    )?;
    lacks(
        &w.ok("alice", &["claims", "lobby"])?,
        "ask:1",
        "the answer left its claim held",
    )?;
    has(
        &w.refused("carol", &["answer", "lobby", "1", "again"])?,
        "#1 is not an open ask",
        "answering a closed ask",
    )?;
    Ok(vec![])
}

fn find(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("alice", &["join", "lobby"])?;
    w.ok(
        "alice",
        &["ask", "lobby", "which port does the gateway listen on"],
    )?;
    w.ok("bob", &["answer", "lobby", "1", "7878"])?;
    w.ok("alice", &["ask", "lobby", "who owns the release pipeline"])?;
    w.ok("carol", &["join", "lobby"])?;
    let found = w.run("carol", &["find", "lobby", "gateway listen"]);
    check(found.ok, format!("find failed: {}", found.stderr))?;
    has(&found.stdout, "7878", "the answer")?;
    has(&found.stdout, "answered by bob", "who answered")?;
    has(
        &found.stderr,
        "data from other agents",
        "the data rule is stated",
    )?;
    // an ask nobody answered is not found
    let none = w.run("carol", &["find", "lobby", "release pipeline"]);
    check(none.stdout.is_empty(), "find printed an unanswered ask")?;
    has(&none.stderr, "no answered ask", "the empty result says so")?;
    // read only: carol's cursor did not move and nothing was posted
    has(
        &w.ok("carol", &["read", "lobby", "--peek"])?,
        "release pipeline",
        "find moved the cursor",
    )?;
    check(
        w.ok("carol", &["history", "lobby", "--last", "50"])?
            .lines()
            .count()
            == 3,
        "find posted a message",
    )?;
    Ok(vec![])
}

fn reopen(w: &World) -> Res {
    w.set_clock(T0);
    for who in ["alice", "bob"] {
        w.ok(who, &["join", "lobby"])?;
    }
    w.ok("alice", &["ask", "lobby", "how do I rotate the key"])?;
    w.ok("bob", &["answer", "lobby", "1", "just do it"])?;
    lacks(
        &w.ok("bob", &["asks", "lobby"])?,
        "rotate the key",
        "an answered ask is open",
    )?;
    // someone else's reopen counts for nothing
    w.ok("bob", &["post", "lobby", "reopen #1: not enough"])?;
    lacks(
        &w.ok("bob", &["asks", "lobby"])?,
        "rotate the key",
        "a reopen by a non-asker reopened the ask",
    )?;
    // the asker's reopen brings it back
    w.ok("alice", &["post", "lobby", "reopen #1: no steps given"])?;
    has(
        &w.ok("bob", &["asks", "lobby"])?,
        "rotate the key",
        "the asker's reopen",
    )?;
    // a second answer ends it again
    w.ok("bob", &["answer", "lobby", "1", "run admin rotate-key"])?;
    lacks(
        &w.ok("bob", &["asks", "lobby"])?,
        "rotate the key",
        "a second answer left it open",
    )?;
    Ok(vec![])
}
