//! The machine-derived member status (#1403): the opt-in PostToolUse and Stop hooks set the status shown by `who` and
//! the board from activity, so agents need not post `working:` and `parked:` lines. Fixture hook inputs run through
//! the real binary. The tool events carry a secret-looking sentinel in the command and the file content; it must
//! never be stored or shown.
mod common;

use common::*;

const SECRET: &str = "SENTINEL-sk-live-4242";

fn project(h: &Home) -> String {
    let p = h.0.join("proj");
    std::fs::create_dir_all(p.join(".git")).unwrap();
    p.to_str().unwrap().to_string()
}

fn run(h: &Home, who: &str, event: &str, extra: &str, flag: bool) {
    let proj = project(h);
    let payload =
        format!(r#"{{"session_id":"s-{who}","hook_event_name":"{event}","cwd":"{proj}"{extra}}}"#);
    let mut args = vec!["hook", "--event", event];
    if flag {
        args.push("--machine-status");
    }
    let o = hook_with_payload(h, Some(who), &payload, &args);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

fn tool(h: &Home, who: &str, flag: bool) {
    run(
        h,
        who,
        "PostToolUse",
        &format!(
            r#","tool_name":"Bash","tool_input":{{"command":"curl -H 'Authorization: {SECRET}' https://x"}},"tool_response":"{SECRET}""#
        ),
        flag,
    );
}

fn stop(h: &Home, who: &str, extra: &str) {
    run(h, who, "Stop", extra, true);
}

/// The status of `who` as bob sees it through `who` and the board.
fn seen(h: &Home, member: &str) -> String {
    let who = h.ok("bob", &["who", "lobby"]);
    let line = who
        .lines()
        .find(|l| l.starts_with(&format!("{member}\t")))
        .unwrap_or_else(|| panic!("no {member} in: {who}"))
        .to_string();
    let board = h.ok("bob", &["board", "lobby"]);
    format!("{line}\n{board}")
}

fn status_text(h: &Home, member: &str) -> Option<String> {
    let who = h.ok("bob", &["who", "lobby"]);
    let line = who
        .lines()
        .find(|l| l.starts_with(&format!("{member}\t")))?;
    line.split('\t')
        .find_map(|p| p.strip_prefix("status: "))
        .map(String::from)
}

fn setup(h: &Home) {
    h.ok("alice", &["join", "lobby"]);
    h.ok("bob", &["join", "lobby"]);
}

#[test]
fn the_first_tool_use_sets_working_marked_auto_on_who_and_the_board_and_posts_no_message() {
    let h = Home::new();
    setup(&h);
    let before = h.ok("bob", &["read", "lobby", "--peek"]);
    tool(&h, "alice", true);
    let s = seen(&h, "alice");
    assert!(
        status_text(&h, "alice").is_some_and(|t| t.starts_with("working (auto)")),
        "{s}"
    );
    assert!(s.contains("working (auto)"), "the board shows it too: {s}");
    assert_eq!(
        h.ok("bob", &["read", "lobby", "--peek"]),
        before,
        "the machine status posts no message"
    );
    assert!(!s.contains(SECRET) && !s.contains("curl"), "{s}");
}

#[test]
fn without_the_flag_nothing_is_written() {
    let h = Home::new();
    setup(&h);
    tool(&h, "alice", false);
    run(&h, "alice", "Stop", "", false);
    assert_eq!(status_text(&h, "alice"), None, "{}", seen(&h, "alice"));
}

#[test]
fn stop_sets_idle_and_a_declared_long_wait_sets_parked() {
    let h = Home::new();
    setup(&h);
    tool(&h, "alice", true);
    stop(&h, "alice", "");
    assert!(
        status_text(&h, "alice").is_some_and(|t| t.starts_with("idle (auto)")),
        "{}",
        seen(&h, "alice")
    );
    tool(&h, "alice", true);
    assert!(
        status_text(&h, "alice").is_some_and(|t| t.starts_with("working (auto)")),
        "the first tool call after idle sets working again"
    );
    stop(&h, "alice", r#","background_tasks":[{"id":"b1"}]"#);
    assert!(
        status_text(&h, "alice").is_some_and(|t| t.starts_with("parked (auto)")),
        "{}",
        seen(&h, "alice")
    );
}

#[test]
fn a_status_the_member_set_stays_through_tool_use_and_stop() {
    let h = Home::new();
    setup(&h);
    h.ok(
        "alice",
        &["status", "lobby", "parked: waiting on CI, back by 2h"],
    );
    tool(&h, "alice", true);
    stop(&h, "alice", "");
    tool(&h, "alice", true);
    assert!(
        status_text(&h, "alice").is_some_and(|t| t.starts_with("parked: waiting on CI")),
        "{}",
        seen(&h, "alice")
    );
}

#[test]
fn a_session_end_marks_done_once_with_or_without_the_machine_status_and_the_machine_never_posts() {
    let h = Home::new();
    setup(&h);
    tool(&h, "alice", true);
    let before = h.ok("bob", &["read", "lobby", "--peek"]);
    let payload = r#"{"session_id":"s-alice","hook_event_name":"SessionEnd","reason":"other"}"#;
    let o = hook_with_payload(
        &h,
        Some("alice"),
        payload,
        &["hook", "--event", "SessionEnd"],
    );
    assert!(o.status.success());
    let shown = status_text(&h, "alice").unwrap_or_default();
    assert!(shown.starts_with("done session ended"), "{shown}");
    assert!(!shown.contains("(auto)"), "{shown}");
    assert_eq!(h.ok("bob", &["read", "lobby", "--peek"]), before);
}

#[test]
fn a_tool_call_made_inside_a_subagent_is_not_the_parents_activity() {
    let h = Home::new();
    setup(&h);
    run(
        &h,
        "alice",
        "PostToolUse",
        r#","agent_id":"sub-1","tool_name":"Edit","tool_input":{"file_path":"x"}"#,
        true,
    );
    assert_eq!(status_text(&h, "alice"), None);
}

#[test]
fn a_burst_of_tool_calls_does_not_change_the_status_age_inside_the_window() {
    let h = Home::new();
    setup(&h);
    tool(&h, "alice", true);
    let first = status_text(&h, "alice").unwrap();
    for _ in 0..10 {
        tool(&h, "alice", true);
    }
    assert_eq!(status_text(&h, "alice").unwrap(), first);
}

/// The room messages a scripted session needs for the same observable state: four phases of work with a long wait and
/// an end. Without the machine status the agent announces each change itself (the convention words); with it the
/// hooks do, so only the two messages with content remain.
fn scripted_session(machine: bool) -> (usize, Vec<String>) {
    let h = Home::new();
    setup(&h);
    let mut states = Vec::new();
    let mut announce = |text: &str| {
        if !machine {
            h.ok("alice", &["post", "lobby", text]);
        }
    };
    h.ok(
        "alice",
        &["post", "lobby", "found a gap in the grammar, taking it"],
    );
    announce("working: grammar gap");
    tool(&h, "alice", machine);
    states.push(status_text(&h, "alice").unwrap_or_default());
    announce("checkpoint: grammar fixed, resume at tests");
    tool(&h, "alice", machine);
    announce("working: tests");
    tool(&h, "alice", machine);
    announce("parked: waiting on CI, back by 1h");
    if machine {
        stop(&h, "alice", r#","background_tasks":[{"id":"ci"}]"#);
    }
    states.push(status_text(&h, "alice").unwrap_or_default());
    announce("working: CI green");
    if machine {
        tool(&h, "alice", true);
    }
    announce("checkpoint: CI green, resume at the merge");
    h.ok("alice", &["post", "lobby", "grammar gap fixed and merged"]);
    if machine {
        stop(&h, "alice", "");
    }
    states.push(status_text(&h, "alice").unwrap_or_default());
    let log = h.ok("bob", &["read", "lobby", "--peek"]);
    let mine = log.lines().filter(|l| l.contains("alice")).count();
    (mine, states)
}

#[test]
fn a_scripted_session_needs_far_fewer_room_messages_with_the_machine_status() {
    let (without, _) = scripted_session(false);
    let (with, states) = scripted_session(true);
    println!("room messages for the scripted session: without {without}, with {with}");
    assert_eq!(with, 2, "only the two messages with content remain");
    assert!(without >= 8, "{without}");
    assert!(
        states[0].starts_with("working (auto)")
            && states[1].starts_with("parked (auto)")
            && states[2].starts_with("idle (auto)"),
        "the state a peer sees followed the session: {states:?}"
    );
}
