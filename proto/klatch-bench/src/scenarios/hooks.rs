use super::*;
use std::io::Write;

pub(super) fn all() -> Vec<Scenario> {
    vec![
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
