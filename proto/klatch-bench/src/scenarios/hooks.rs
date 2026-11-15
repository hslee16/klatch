use super::*;
use std::io::Write;

pub(super) fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "convention_prints_a_brief_a_parent_can_paste_for_a_child",
            origin: "#491",
            layer: "functional",
            run: convention_brief,
        },
        Scenario {
            name: "a_claimed_path_stops_a_second_editor_and_a_broken_store_never_stops_anyone",
            origin: "#33, #34, R19, DESIGN D5",
            layer: "fault",
            run: claim_hook,
        },
        Scenario {
            name: "an_idle_session_is_woken_by_a_pointer_that_carries_no_message_text",
            origin: "#37, #39, DESIGN section 9",
            layer: "fault",
            run: wake,
        },
        Scenario {
            name: "a_long_digest_stays_under_claude_codes_hook_output_cap_and_counts_what_it_cuts",
            origin: "#549, RESEARCH-NOTES 48.2",
            layer: "functional",
            run: digest_budget,
        },
        Scenario {
            name: "a_claim_held_two_hours_reminds_its_owner_once_and_tells_a_peer_its_age",
            origin: "#547, RESEARCH-NOTES 49.2",
            layer: "functional",
            run: claim_age,
        },
        Scenario {
            name: "a_hook_inside_a_subagent_prints_nothing_and_keeps_the_parents_mail",
            origin: "#493",
            layer: "fault",
            run: subagent_hook,
        },
    ]
}

/// A tool call as Claude Code hands it to a `PreToolUse` hook.
fn tool_call(tool: &str, file: &str) -> String {
    serde_json::json!({
        "session_id": "s", "hook_event_name": "PreToolUse", "tool_name": tool,
        "tool_input": {"file_path": file}, "cwd": "/w"
    })
    .to_string()
}

/// Run the hook as `who` with `stdin`: its exit code, standard output and standard error.
fn pre_tool_use(
    w: &World,
    who: &str,
    stdin: &str,
) -> Result<(Option<i32>, String, String), String> {
    let mut child = w
        .spawn_piped(who, &["hook", "--event", "PreToolUse"])
        .map_err(|e| e.to_string())?;
    child
        .stdin
        .take()
        .ok_or("no stdin")?
        .write_all(stdin.as_bytes())
        .map_err(|e| e.to_string())?;
    let o = child.wait_with_output().map_err(|e| e.to_string())?;
    Ok((
        o.status.code(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    ))
}

fn claim_hook(w: &World) -> Res {
    w.set_clock(T0);
    for who in ["a", "b"] {
        w.ok(who, &["join", "r"])?;
    }
    w.ok(
        "a",
        &["claim", "r", "/w/src/api.rs", "--reason", "refactor"],
    )?;
    let (code, out, err) = pre_tool_use(w, "b", &tool_call("Edit", "/w/src/api.rs"))?;
    check(
        code == Some(2),
        format!("a claimed edit was not blocked: {code:?}"),
    )?;
    has(&err, "claimed by a", "the reason names the holder")?;
    check(out.is_empty(), "a block printed on standard output")?;
    let (code, ..) = pre_tool_use(w, "a", &tool_call("Edit", "/w/src/api.rs"))?;
    check(code == Some(0), "the holder was blocked from its own path")?;
    let (code, ..) = pre_tool_use(w, "b", &tool_call("Edit", "/w/src/other.rs"))?;
    check(code == Some(0), "an unclaimed path was blocked")?;
    w.advance(10_000);
    let (code, ..) = pre_tool_use(w, "b", &tool_call("Edit", "/w/src/api.rs"))?;
    check(code == Some(0), "a lapsed claim still blocked")?;
    // a store that cannot be read fails open, with a line the model reads
    w.ok("a", &["claim", "r", "/w/src/api.rs", "--reason", "again"])?;
    std::fs::write(w.home.join("rooms.db"), b"not a database").map_err(|e| e.to_string())?;
    let (code, out, _) = pre_tool_use(w, "b", &tool_call("Edit", "/w/src/api.rs"))?;
    check(code == Some(0), "a broken store blocked an edit")?;
    has(&out, "could not check claims", "the failure was silent")?;
    Ok(vec![])
}

fn wake(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("sleeper", &["join", "r"])?;
    let mut watcher = w
        .spawn("sleeper", &["wake", "--timeout", "30"])
        .map_err(|e| e.to_string())?;
    std::thread::sleep(std::time::Duration::from_millis(500));
    check(
        watcher.try_wait().map_err(|e| e.to_string())?.is_none(),
        "the watcher ended with nothing to say",
    )?;
    w.ok("peer", &["post", "r", "the launch code is hunter2"])?;
    let o = watcher.wait_with_output().map_err(|e| e.to_string())?;
    check(
        o.status.code() == Some(2),
        format!("did not wake: {:?}", o.status.code()),
    )?;
    let err = String::from_utf8_lossy(&o.stderr).into_owned();
    has(&err, "--discover read r", "the pointer names the command")?;
    lacks(&err, "hunter2", "the pointer carried the message")?;
    lacks(&err, "peer", "the pointer named the sender")?;
    // the message is still unread, for the hook to show
    has(
        &w.ok("sleeper", &["read", "r"])?,
        "hunter2",
        "the watcher consumed the message",
    )?;
    Ok(vec![])
}

/// A `PostToolUse` hook as `who`, with the session id and, for a subagent's tool call, an agent id.
fn post_tool_use(w: &World, who: &str, agent: Option<&str>) -> Result<String, String> {
    let mut input = serde_json::json!({"session_id": "s", "hook_event_name": "PostToolUse"});
    if let Some(a) = agent {
        input["agent_id"] = a.into();
    }
    let mut child = w
        .spawn_piped(who, &["hook", "--event", "PostToolUse"])
        .map_err(|e| e.to_string())?;
    child
        .stdin
        .take()
        .ok_or("no stdin")?
        .write_all(input.to_string().as_bytes())
        .map_err(|e| e.to_string())?;
    let o = child.wait_with_output().map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

fn subagent_hook(w: &World) -> Res {
    w.set_clock(T0);
    for who in ["parent", "peer"] {
        w.ok(who, &["join", "r"])?;
    }
    w.ok("peer", &["post", "r", "for the parent"])?;
    let sub = post_tool_use(w, "parent", Some("agent-1"))?;
    check(sub.is_empty(), "a subagent's hook printed mail")?;
    let own = post_tool_use(w, "parent", None)?;
    has(
        &own,
        "for the parent",
        "the subagent's hook ate the parent's mail",
    )?;
    Ok(vec![])
}

fn convention_brief(w: &World) -> Res {
    let t = w.ok("parent", &["convention", "--name", "kid"])?;
    has(&t, "KLATCH_NAME=kid", "the child's name is on its commands")?;
    has(&t, "--discover join lobby", "the brief says how to join")?;
    has(&t, "[klatch convention v1]", "the convention is included")?;
    Ok(vec![])
}

/// Claude Code keeps only a 2 KB preview of hook output near 9.8 KB (measured); a digest must stay well under it.
fn digest_budget(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("reader", &["join", "r"])?;
    for i in 0..15 {
        // distinct bodies: an identical repeat from one member is refused
        w.ok(
            "peer",
            &["post", "r", &format!("{i:02} {}", "z".repeat(900))],
        )?;
    }
    let mut child = w
        .spawn_piped("reader", &["hook", "--event", "UserPromptSubmit"])
        .map_err(|e| e.to_string())?;
    child
        .stdin
        .take()
        .ok_or("no stdin")?
        .write_all(br#"{"session_id":"s","hook_event_name":"UserPromptSubmit"}"#)
        .map_err(|e| e.to_string())?;
    let o = child.wait_with_output().map_err(|e| e.to_string())?;
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    check(
        out.len() < 9_000,
        format!("the hook printed {} bytes", out.len()),
    )?;
    has(
        &out,
        "more messages not shown",
        "the cut messages are not counted",
    )?;
    has(
        &out,
        "klatch read r --peek",
        "the line does not say how to read them",
    )?;
    Ok(vec![])
}

/// An owner that keeps working renews its claim forever; at two hours the hook says so, once, and a peer refused
/// by the claim is told how long it has been held.
fn claim_age(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("holder", &["join", "r"])?;
    w.ok("other", &["join", "r"])?;
    w.ok("holder", &["claim", "r", "src/a.rs", "--ttl", "3600"])?;
    for _ in 0..11 {
        w.advance(600);
        let early = post_tool_use(w, "holder", None)?;
        check(
            !early.contains("has been held"),
            "reminded before two hours",
        )?;
    }
    w.advance(600);
    let at_two = post_tool_use(w, "holder", None)?;
    has(
        &at_two,
        "your claim on src/a.rs has been held for 2h",
        "no reminder at two hours",
    )?;
    let again = post_tool_use(w, "holder", None)?;
    check(!again.contains("has been held"), "the reminder repeated")?;
    has(
        &w.refused("other", &["claim", "r", "src/a.rs"])?,
        "held for 2h",
        "the peer was not told the age",
    )?;
    Ok(vec![])
}
