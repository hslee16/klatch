//! What a requester and a reader see of crossings and of a room's hidden rows (#516, #679, #961, #750, #1008).
use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "a_requester_lists_its_own_crossings_over_the_command_wire_and_digest_and_never_sees_text",
            origin: "#516, #679, #961",
            layer: "functional",
            run: crossings,
        },
        Scenario {
            name: "a_librarian_that_waits_does_not_join_the_room_or_move_its_place",
            origin: "#750",
            layer: "functional",
            run: librarian_wait,
        },
        Scenario {
            name: "history_last_n_is_filled_with_visible_messages_past_a_planted_canary",
            origin: "#1008, #954",
            layer: "functional",
            run: history_window,
        },
    ]
}

fn crossings(w: &World) -> Res {
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
    // #1 of a private room is its canary, so the post is #2
    has(
        &w.ok("alice", &["crossings"])?,
        "no crossings",
        "an empty listing",
    )?;
    w.ok("alice", &["join", "open"])?;
    w.ok("bob", &["join", "open"])?;
    w.ok("alice", &["cross", "vault", "2", "--to", "open"])?;
    let list = w.ok("alice", &["crossings"])?;
    has(&list, "vault#2", "the pointer")?;
    has(&list, "waiting", "the state")?;
    lacks(&list, "blue", "the listing showed the text")?;
    // another member sees none of it
    has(
        &w.ok("bob", &["crossings"])?,
        "no crossings",
        "a member listed another's crossings",
    )?;
    // the same rows over the wire: a state and a fixed sentence, never the text
    let req = serde_json::json!({"v": 1, "type": "request", "id": "1", "method": "my_crossings", "params": {}})
        .to_string();
    let lines = w.serve("alice", &[req.clone()])?;
    let wire = lines.join("\n");
    has(&wire, "\"ok\":true", "the wire call")?;
    has(&wire, "\"state\":\"waiting\"", "the wire state")?;
    lacks(&wire, "blue", "the wire carried the text")?;
    let other = w.serve("bob", &[req])?.join("\n");
    has(
        &other,
        "\"crossings\":[]",
        "the wire listed another member's rows",
    )?;
    let note = hook_digest(w, "alice")?;
    has(
        &note,
        "your crossing of message 2 to open: waiting for a decision",
        "the requester is told the request waits",
    )?;
    lacks(&note, "blue", "the notice carried the text")?;
    lacks(
        &hook_digest(w, "alice")?,
        "waiting for a decision",
        "the waiting notice was repeated",
    )?;
    // nothing was posted into the target room
    lacks(
        &w.ok("bob", &["read", "open"])?,
        "blue",
        "text crossed without a verdict",
    )?;
    Ok(vec![])
}

fn librarian_wait(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("worker", &["join", "town"])?;
    w.ok("worker", &["post", "town", "the build is green"])?;
    check(
        w.admin(&["librarian", "lib"], "").ok,
        "granting the role failed",
    )?;
    // waiting returns what is there, twice: the first wait moved no position
    for round in 1..=2 {
        has(
            &w.ok("lib", &["wait", "town", "--timeout", "0", "--peek"])?,
            "the build is green",
            &format!("librarian wait, round {round}"),
        )?;
    }
    // and it never became a member of the room
    lacks(
        &w.ok("worker", &["who", "town"])?,
        "lib",
        "waiting made the librarian a member",
    )?;
    Ok(vec![])
}

fn history_window(w: &World) -> Res {
    w.set_clock(T0);
    check(
        w.admin(&["label", "vault", "private"], "").ok,
        "label failed",
    )?;
    check(
        w.admin(&["clear", "priv", "private"], "").ok,
        "clear failed",
    )?;
    w.ok("priv", &["join", "vault"])?;
    for i in 1..=5 {
        w.ok("priv", &["post", "vault", &format!("note {i}")])?;
    }
    // the newest rows are canaries, which history never shows
    for _ in 0..3 {
        check(w.admin(&["canary", "vault"], "").ok, "canary failed")?;
    }
    let last = w.ok("priv", &["history", "vault", "--last", "3"])?;
    lacks(&last, "kc-canary-", "history showed a canary")?;
    for n in ["note 3", "note 4", "note 5"] {
        has(&last, n, "the window was not filled with visible messages")?;
    }
    check(
        !last.contains("note 2"),
        "the window ran past the three asked for",
    )?;
    Ok(vec![])
}

/// The text a UserPromptSubmit hook prints for `who`.
fn hook_digest(w: &World, who: &str) -> Result<String, String> {
    use std::io::Write;
    let input = r#"{"session_id":"s","hook_event_name":"UserPromptSubmit","prompt":"x"}"#;
    let mut c = w
        .spawn_piped(who, &["hook", "--event", "UserPromptSubmit"])
        .map_err(|e| e.to_string())?;
    c.stdin
        .take()
        .ok_or("no stdin")?
        .write_all(input.as_bytes())
        .map_err(|e| e.to_string())?;
    let o = c.wait_with_output().map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr))
}
