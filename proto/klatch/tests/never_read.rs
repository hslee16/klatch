//! A member that posts without ever reading the room (#1411): how it happens, and the one-time notice that answers it.
//! The first group reproduces the causes with scripted members against a throwaway home (these describe today's
//! behaviour and pass); the second group is the fix.
mod common;

use common::*;

fn stderr(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn unread(h: &Home, who: &str) -> usize {
    h.ok(who, &["read", "lobby", "--count"])
        .trim()
        .parse()
        .unwrap()
}

/// A lobby that already has a conversation in it.
fn busy() -> Home {
    let h = Home::new();
    h.ok("old", &["join", "lobby"]);
    h.ok("old", &["post", "lobby", "first"]);
    h.ok("old", &["post", "lobby", "second"]);
    h
}

// ---- reproductions of the causes -------------------------------------------------------------------------------

#[test]
fn cause_post_without_a_join_makes_a_member_with_nothing_read() {
    let h = busy();
    h.ok("worker", &["post", "lobby", "ready"]);
    assert!(h.ok("worker", &["who", "lobby"]).contains("worker"));
    assert_eq!(
        unread(&h, "worker"),
        2,
        "the post made a row; nothing was read"
    );
}

#[test]
fn cause_a_plain_join_does_not_advance_the_read_cursor() {
    let h = busy();
    h.ok("worker", &["join", "lobby"]);
    h.ok("worker", &["post", "lobby", "ready"]);
    assert_eq!(unread(&h, "worker"), 2);
}

#[test]
fn not_a_cause_a_new_session_start_acknowledges_the_history() {
    let h = busy();
    h.ok("fresh", &["hook", "--event", "SessionStart"]);
    h.ok("fresh", &["post", "lobby", "ready"]);
    assert_eq!(
        unread(&h, "fresh"),
        0,
        "the session start moved the cursor to the head"
    );
}

#[test]
fn cause_a_subagent_tool_call_gets_no_digest() {
    let h = busy();
    h.ok("parent", &["join", "lobby"]);
    let o = hook_with_payload(
        &h,
        Some("parent"),
        r#"{"agent_id":"sub-1","tool_name":"Bash"}"#,
        &["hook", "--event", "PostToolUse"],
    );
    assert!(o.status.success());
    assert_eq!(
        String::from_utf8_lossy(&o.stdout).trim(),
        "",
        "a subagent call stays silent"
    );
}

#[test]
fn a_digest_shown_by_a_hook_leaves_the_read_cursor_where_it_was() {
    let h = busy();
    h.ok("worker", &["join", "lobby"]);
    let shown = h.ok("worker", &["hook", "--event", "UserPromptSubmit"]);
    assert!(
        shown.contains("second"),
        "the digest showed the room: {shown}"
    );
    assert_eq!(
        unread(&h, "worker"),
        2,
        "so a cursor count calls this member a non-reader"
    );
}

// ---- the fix ---------------------------------------------------------------------------------------------------

#[test]
fn a_first_post_without_reading_gets_a_one_line_notice_with_the_read_command_once() {
    let h = busy();
    let first = stderr(&h.run("worker", &["post", "lobby", "ready"]));
    let notice: Vec<&str> = first
        .lines()
        .filter(|l| l.contains("klatch read lobby"))
        .collect();
    assert_eq!(notice.len(), 1, "{first}");
    assert!(notice[0].contains("2 unread"), "{first}");
    assert!(
        notice[0].contains("DATA"),
        "the notice marks messages as data: {first}"
    );
    let second = stderr(&h.run("worker", &["post", "lobby", "again"]));
    assert!(!second.contains("klatch read lobby"), "once only: {second}");
}

#[test]
fn an_ask_without_reading_gets_the_notice_too() {
    let h = busy();
    let o = h.run("worker", &["ask", "lobby", "who owns the parser?"]);
    assert!(stderr(&o).contains("klatch read lobby"), "{}", stderr(&o));
}

#[test]
fn reading_first_means_no_notice() {
    let h = busy();
    h.ok("worker", &["join", "lobby"]);
    h.ok("worker", &["read", "lobby"]);
    let o = h.run("worker", &["post", "lobby", "ready"]);
    assert!(!stderr(&o).contains("klatch read"), "{}", stderr(&o));
}

#[test]
fn a_member_whose_hook_showed_the_room_gets_no_notice() {
    let h = busy();
    h.ok("worker", &["join", "lobby"]);
    h.ok("worker", &["hook", "--event", "UserPromptSubmit"]);
    let o = h.run("worker", &["post", "lobby", "ready"]);
    assert!(!stderr(&o).contains("klatch read"), "{}", stderr(&o));
}

#[test]
fn a_member_that_started_a_session_gets_no_notice() {
    let h = busy();
    h.ok("fresh", &["hook", "--event", "SessionStart"]);
    let o = h.run("fresh", &["post", "lobby", "ready"]);
    assert!(!stderr(&o).contains("klatch read"), "{}", stderr(&o));
}

#[test]
fn nothing_waiting_means_no_notice() {
    let h = Home::new();
    let o = h.run("first", &["post", "lobby", "hello"]);
    assert!(!stderr(&o).contains("klatch read"), "{}", stderr(&o));
}

#[test]
fn a_member_that_waited_gets_no_notice() {
    let h = busy();
    h.ok("worker", &["join", "lobby"]);
    // a wait that returns at once because mail is waiting
    let _ = h.run("worker", &["wait", "lobby", "--timeout", "1"]);
    let o = h.run("worker", &["post", "lobby", "ready"]);
    assert!(!stderr(&o).contains("klatch read"), "{}", stderr(&o));
}

// ---- klatch dm (#1415) -----------------------------------------------------------------------------------------

#[test]
fn a_first_dm_reply_without_reading_gets_the_notice_once() {
    let h = Home::new();
    h.ok("old", &["dm", "worker", "can", "you", "take", "this"]);
    let first = stderr(&h.run("worker", &["dm", "old", "yes"]));
    let notice: Vec<&str> = first
        .lines()
        .filter(|l| l.contains("klatch read"))
        .collect();
    assert_eq!(notice.len(), 1, "{first}");
    assert!(notice[0].contains("1 unread message"), "{first}");
    assert!(notice[0].contains("DATA"), "{first}");
    assert!(
        !first.contains("can you take this"),
        "never the content: {first}"
    );
    let second = stderr(&h.run("worker", &["dm", "old", "again"]));
    assert!(!second.contains("klatch read"), "once only: {second}");
}

#[test]
fn a_dm_reply_after_reading_the_thread_gets_no_notice() {
    let h = Home::new();
    h.ok("old", &["dm", "worker", "hello"]);
    h.ok("worker", &["read", "dm.old.worker"]);
    let o = h.run("worker", &["dm", "old", "hi back"]);
    assert!(!stderr(&o).contains("klatch read"), "{}", stderr(&o));
}

#[test]
fn a_first_dm_to_someone_with_no_thread_yet_gets_no_notice() {
    let h = Home::new();
    let o = h.run("worker", &["dm", "old", "hello"]);
    assert!(!stderr(&o).contains("klatch read"), "{}", stderr(&o));
}
