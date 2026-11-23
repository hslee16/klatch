//! What a member's own state shows to others: status, mode, grant, and the per-member rate limit (#260, #677, #893,
//! #972, #1023).
use super::*;
use std::io::Write;

pub(super) fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "a_done_status_clears_when_the_member_returns_shows_its_age_and_available_stays",
            origin: "#260, #223",
            layer: "functional",
            run: status,
        },
        Scenario {
            name: "who_shows_each_members_mode_and_the_wake_path_that_mode_gets",
            origin: "#677, #552",
            layer: "functional",
            run: who_mode,
        },
        Scenario {
            name: "only_a_rooms_creator_can_grant_in_it_and_never_to_itself_or_the_top_level",
            origin: "#893, #897",
            layer: "functional",
            run: grant,
        },
        Scenario {
            name: "a_member_limit_overrides_the_global_one_and_the_listing_names_every_limit",
            origin: "#972, #1023",
            layer: "functional",
            run: member_limit,
        },
    ]
}

fn line_of(out: &str, name: &str) -> String {
    out.lines()
        .find(|l| l.starts_with(name))
        .unwrap_or_default()
        .to_string()
}

fn status(w: &World) -> Res {
    w.set_clock(T0);
    for who in ["alice", "bob"] {
        w.ok(who, &["join", "lobby"])?;
    }
    w.ok("alice", &["status", "lobby", "done wrapped up"])?;
    w.advance(180);
    let shown = line_of(&w.ok("bob", &["who", "lobby"])?, "alice");
    has(&shown, "status: done wrapped up (3m ago)", "the age")?;
    // a read is not returning
    w.ok("alice", &["read", "lobby", "--peek"])?;
    has(
        &line_of(&w.ok("bob", &["who", "lobby"])?, "alice"),
        "status: done",
        "a read cleared the status",
    )?;
    // posting again clears the done status
    w.ok("alice", &["post", "lobby", "back with one more thing"])?;
    lacks(
        &line_of(&w.ok("bob", &["who", "lobby"])?, "alice"),
        "status:",
        "a post did not clear done",
    )?;
    // available is not done: it stays through a post
    w.ok("alice", &["status", "lobby", "available"])?;
    w.ok("alice", &["post", "lobby", "free for work"])?;
    has(
        &line_of(&w.ok("bob", &["who", "lobby"])?, "alice"),
        "status: available",
        "available was cleared by a post",
    )?;
    // another status word is free text and stays too
    w.ok("bob", &["status", "lobby", "busy reviewing"])?;
    w.ok("bob", &["post", "lobby", "still here"])?;
    has(
        &line_of(&w.ok("alice", &["who", "lobby"])?, "bob"),
        "status: busy reviewing",
        "a free text status was cleared",
    )?;
    Ok(vec![])
}

fn who_mode(w: &World) -> Res {
    w.set_clock(T0);
    for who in ["fast", "careful", "quiet"] {
        w.ok(who, &["join", "r"])?;
    }
    for (who, mode) in [("fast", "bypassPermissions"), ("careful", "default")] {
        let input = serde_json::json!({"session_id": "s", "hook_event_name": "PostToolUse", "permission_mode": mode});
        let mut c = w
            .spawn_piped(who, &["hook", "--event", "PostToolUse"])
            .map_err(|e| e.to_string())?;
        c.stdin
            .take()
            .ok_or("no stdin")?
            .write_all(input.to_string().as_bytes())
            .map_err(|e| e.to_string())?;
        c.wait_with_output().map_err(|e| e.to_string())?;
    }
    let out = w.ok("quiet", &["who", "r"])?;
    has(
        &line_of(&out, "fast"),
        "mode: bypassPermissions (wake: hook)",
        "a bypassing member is woken by its hook",
    )?;
    has(
        &line_of(&out, "careful"),
        "mode: default (wake: nudge)",
        "a prompting member may be nudged",
    )?;
    has(
        &line_of(&out, "quiet"),
        "mode: unknown (wake: hook)",
        "a member whose hooks never ran",
    )?;
    Ok(vec![])
}

fn grant(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("alice", &["join", "design"])?;
    w.ok("bob", &["join", "design"])?;
    w.ok("bob", &["join", "mine"])?;
    let said = w.ok("alice", &["grant", "design", "bob", "public"])?;
    has(&said, "bob", "the grantor names who")?;
    has(&said, "design", "the grantor names the room")?;
    has(
        &w.refused("bob", &["grant", "design", "alice", "public"])?,
        "grantor",
        "a non-grantor granted",
    )?;
    has(
        &w.refused("alice", &["grant", "mine", "bob", "public"])?,
        "grantor",
        "a member granted in a room it only joined",
    )?;
    has(
        &w.refused("alice", &["grant", "design", "alice", "public"])?,
        "itself",
        "a grantor granted itself",
    )?;
    has(
        &w.refused("alice", &["grant", "design", "bob", "secret"])?,
        "top level",
        "the top level was granted",
    )?;
    Ok(vec![])
}

/// Set a global limit and two member limits (one lower, one higher); the listing shows them in order.
fn set_and_list_limits(w: &World) -> Result<(), String> {
    for who in ["noisy", "trusted", "plain"] {
        w.ok(who, &["join", "r"])?;
    }
    has(
        &w.admin(&["rate-limit", "--list"], "").stdout,
        "no rate limit",
        "an empty listing",
    )?;
    check(w.admin(&["rate-limit", "5", "60"], "").ok, "global limit")?;
    check(
        w.admin(&["rate-limit", "--member", "noisy", "2", "60"], "")
            .ok,
        "member limit (lower)",
    )?;
    check(
        w.admin(&["rate-limit", "--member", "trusted", "9", "60"], "")
            .ok,
        "member limit (higher)",
    )?;
    let list = w.admin(&["rate-limit", "--list"], "").stdout;
    let lines: Vec<&str> = list.lines().collect();
    check(
        lines.len() == 3,
        format!("expected three lines, got:\n{list}"),
    )?;
    has(
        lines[0],
        "global\t5 posts in 60 seconds",
        "the global line is first",
    )?;
    has(
        lines[1],
        "member noisy\t2 posts in 60 seconds",
        "members sort by name",
    )?;
    has(
        lines[2],
        "member trusted\t9 posts in 60 seconds",
        "the higher member limit",
    )
}

/// Each member is held to its own limit: the noisy one to its lower one, trusted to its higher one, plain to the
/// global one.
fn limits_hold_per_member(w: &World) -> Result<(), String> {
    for i in 0..2 {
        w.advance(1);
        w.ok("noisy", &["post", "r", &format!("n{i}")])?;
    }
    w.advance(1);
    has(
        &w.refused("noisy", &["post", "r", "n2"])?,
        "rate-limit",
        "the member's own lower limit",
    )?;
    // trusted is held to its own higher limit, not the global five
    for i in 0..6 {
        w.advance(1);
        w.ok("trusted", &["post", "r", &format!("t{i}")])?;
    }
    // plain is held to the global limit
    for i in 0..5 {
        w.advance(1);
        w.ok("plain", &["post", "r", &format!("p{i}")])?;
    }
    w.advance(1);
    has(
        &w.refused("plain", &["post", "r", "p5"])?,
        "rate-limit",
        "the global limit",
    )
}

/// Lifting a member limit removes it from the listing.
fn lifting_a_limit(w: &World) -> Result<(), String> {
    check(
        w.admin(&["rate-limit", "--member", "noisy", "--off"], "")
            .ok,
        "lifting the member limit",
    )?;
    lacks(
        &w.admin(&["rate-limit", "--list"], "").stdout,
        "noisy",
        "a lifted limit is still listed",
    )
}

fn member_limit(w: &World) -> Res {
    w.set_clock(T0);
    set_and_list_limits(w)?;
    limits_hold_per_member(w)?;
    lifting_a_limit(w)?;
    Ok(vec![])
}
