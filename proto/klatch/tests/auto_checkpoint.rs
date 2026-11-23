//! The automatic machine checkpoint (#1408): the opt-in Stop and SessionEnd step writes a compact record of where a
//! member's session stands beside its durable task. Fixture hook inputs run through the real binary. The tool events
//! carry a secret-looking sentinel in the command and in the file content; it must never be stored or shown.
mod common;

use common::*;

const SECRET: &str = "SENTINEL-sk-live-4242";

/// A project directory with a `.git` entry, so paths are taken relative to it.
fn project(h: &Home) -> String {
    let p = h.0.join("proj");
    std::fs::create_dir_all(p.join(".git")).unwrap();
    p.to_str().unwrap().to_string()
}

fn tool(h: &Home, who: &str, proj: &str, body: &str, flag: bool) {
    let payload = format!(
        r#"{{"session_id":"s-{who}","hook_event_name":"PostToolUse","cwd":"{proj}",{body}}}"#
    );
    let mut args = vec!["hook", "--event", "PostToolUse"];
    if flag {
        args.push("--checkpoint");
    }
    let o = hook_with_payload(h, Some(who), &payload, &args);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

fn edit(h: &Home, who: &str, proj: &str, file: &str) {
    tool(
        h,
        who,
        proj,
        &format!(
            r#""tool_name":"Edit","tool_input":{{"file_path":"{proj}/{file}","old_string":"{SECRET}","new_string":"{SECRET}"}},"tool_response":"{SECRET}""#
        ),
        true,
    );
}

fn stop(h: &Home, who: &str, proj: &str) {
    let payload = format!(r#"{{"session_id":"s-{who}","hook_event_name":"Stop","cwd":"{proj}"}}"#);
    let o = hook_with_payload(
        h,
        Some(who),
        &payload,
        &["hook", "--event", "Stop", "--checkpoint"],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

fn end(h: &Home, who: &str, proj: &str, reason: &str) {
    let payload = format!(
        r#"{{"session_id":"s-{who}","hook_event_name":"SessionEnd","cwd":"{proj}","reason":"{reason}"}}"#
    );
    let o = hook_with_payload(
        h,
        Some(who),
        &payload,
        &["hook", "--event", "SessionEnd", "--checkpoint"],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

fn machine_part(tasks: &str) -> String {
    let i = tasks
        .find("machine checkpoint")
        .unwrap_or_else(|| panic!("no machine checkpoint in: {tasks}"));
    tasks[i..].to_string()
}

#[test]
fn edits_writes_and_commands_are_recorded_as_classes_and_paths_never_the_command_or_the_content() {
    let h = Home::new();
    let p = project(&h);
    h.ok("alice", &["join", "lobby"]);
    h.ok("alice", &["post", "lobby", "working: parser rewrite"]);
    edit(&h, "alice", &p, "src/a.rs");
    tool(
        &h,
        "alice",
        &p,
        &format!(
            r#""tool_name":"Write","tool_input":{{"file_path":"{p}/src/b.rs","content":"{SECRET}"}}"#
        ),
        true,
    );
    tool(
        &h,
        "alice",
        &p,
        &format!(
            r#""tool_name":"Bash","tool_input":{{"command":"curl -H 'Authorization: {SECRET}' https://x"}}"#
        ),
        true,
    );
    tool(
        &h,
        "alice",
        &p,
        &format!(r#""tool_name":"Read","tool_input":{{"file_path":"{p}/src/never-listed.rs"}}"#),
        true,
    );
    stop(&h, "alice", &p);
    let tasks = h.ok("bob", &["tasks", "lobby"]);
    let m = machine_part(&tasks);
    assert!(m.contains("src/a.rs") && m.contains("src/b.rs"), "{tasks}");
    assert!(m.contains("tools edit write run read"), "{tasks}");
    assert!(
        !m.contains("never-listed"),
        "a read is a class only: {tasks}"
    );
    assert!(!m.contains(&p), "paths are repo-relative: {tasks}");
    for shown in [
        tasks.as_str(),
        &h.ok("bob", &["read", "lobby", "--peek"]),
        &h.ok("bob", &["board", "lobby"]),
    ] {
        assert!(!shown.contains(SECRET), "a sentinel leaked: {shown}");
        assert!(!shown.contains("curl"), "a command line leaked: {shown}");
    }
}

#[test]
fn nothing_is_recorded_unless_the_hook_carries_the_checkpoint_consent() {
    let h = Home::new();
    let p = project(&h);
    h.ok("alice", &["join", "lobby"]);
    h.ok("alice", &["post", "lobby", "working: parser rewrite"]);
    tool(
        &h,
        "alice",
        &p,
        &format!(r#""tool_name":"Edit","tool_input":{{"file_path":"{p}/src/a.rs"}}"#),
        false,
    );
    let payload = format!(r#"{{"session_id":"s-alice","hook_event_name":"Stop","cwd":"{p}"}}"#);
    hook_with_payload(&h, Some("alice"), &payload, &["hook", "--event", "Stop"]);
    let tasks = h.ok("bob", &["tasks", "lobby"]);
    assert!(!tasks.contains("machine checkpoint"), "{tasks}");
}

#[test]
fn a_stop_writes_a_checkpoint_within_the_byte_cap_and_the_next_one_replaces_it() {
    let h = Home::new();
    let p = project(&h);
    h.ok("alice", &["join", "lobby"]);
    h.ok("alice", &["post", "lobby", "working: big change"]);
    for i in 0..40 {
        edit(
            &h,
            "alice",
            &p,
            &format!("crates/some/deep/module/path/file_{i:02}.rs"),
        );
    }
    stop(&h, "alice", &p);
    let first = h.ok("bob", &["tasks", "lobby"]);
    let body = machine_part(&first);
    let text = body.split("machine checkpoint").nth(1).unwrap();
    let text = text
        .split_once(": ")
        .unwrap()
        .1
        .trim_end_matches(')')
        .trim();
    assert!(text.len() <= 600, "{} bytes: {text}", text.len());
    assert!(
        text.contains("file_39.rs") && !text.contains("file_00.rs"),
        "{text}"
    );
    edit(&h, "alice", &p, "src/later.rs");
    stop(&h, "alice", &p);
    let second = h.ok("bob", &["tasks", "lobby"]);
    assert_eq!(second.matches("machine checkpoint").count(), 1, "{second}");
    assert!(second.contains("src/later.rs"), "{second}");
}

#[test]
fn a_session_end_writes_the_final_checkpoint_with_the_claims_it_then_releases() {
    let h = Home::new();
    let p = project(&h);
    h.ok("alice", &["join", "lobby"]);
    h.ok("alice", &["post", "lobby", "working: parser rewrite"]);
    h.ok("alice", &["claim", "lobby", "src/parser.rs"]);
    edit(&h, "alice", &p, "src/parser.rs");
    end(&h, "alice", &p, "clear");
    let claims = h.ok("bob", &["claims", "lobby"]);
    assert!(
        !claims.contains("src/parser.rs"),
        "the claim was released: {claims}"
    );
    let tasks = h.ok("bob", &["tasks", "lobby"]);
    let m = machine_part(&tasks);
    assert!(m.contains("session ended (clear)"), "{tasks}");
    assert!(m.contains("claims src/parser.rs"), "{tasks}");
    assert!(m.contains("files src/parser.rs"), "{tasks}");
    // a reason outside the list is one word, never the text
    let h2 = Home::new();
    let p2 = project(&h2);
    h2.ok("alice", &["join", "lobby"]);
    h2.ok("alice", &["post", "lobby", "working: x"]);
    end(&h2, "alice", &p2, &format!("weird {SECRET}"));
    let t2 = h2.ok("bob", &["tasks", "lobby"]);
    assert!(
        t2.contains("session ended (other)") && !t2.contains(SECRET),
        "{t2}"
    );
}

#[test]
fn an_explicit_checkpoint_wins_and_the_machine_one_is_marked_beneath_it() {
    let h = Home::new();
    let p = project(&h);
    h.ok("alice", &["join", "lobby"]);
    h.ok("alice", &["post", "lobby", "working: parser rewrite"]);
    h.ok(
        "alice",
        &[
            "post",
            "lobby",
            "checkpoint: tokens done, resume at the grammar",
        ],
    );
    edit(&h, "alice", &p, "src/a.rs");
    stop(&h, "alice", &p);
    let tasks = h.ok("bob", &["tasks", "lobby"]);
    let line = tasks.lines().find(|l| l.starts_with("alice:")).unwrap();
    let explicit = line.find("resume at the grammar").unwrap();
    let machine = line.find("machine checkpoint").unwrap();
    assert!(
        explicit < machine,
        "the explicit checkpoint comes first: {line}"
    );
    assert!(line.contains("src/a.rs"), "{line}");
}

#[test]
fn a_member_with_no_task_still_leaves_its_checkpoint_for_a_successor() {
    let h = Home::new();
    let p = project(&h);
    h.ok("alice", &["join", "lobby"]);
    edit(&h, "alice", &p, "src/a.rs");
    end(&h, "alice", &p, "other");
    let tasks = h.ok("bob", &["tasks", "lobby"]);
    assert!(
        tasks.contains("alice") && tasks.contains("src/a.rs"),
        "{tasks}"
    );
    assert!(
        tasks.contains("no task"),
        "it says no task was declared: {tasks}"
    );
}

#[test]
fn a_tool_call_made_inside_a_subagent_is_not_the_parents_work() {
    let h = Home::new();
    let p = project(&h);
    h.ok("alice", &["join", "lobby"]);
    h.ok("alice", &["post", "lobby", "working: x"]);
    tool(
        &h,
        "alice",
        &p,
        &format!(
            r#""agent_id":"sub-1","tool_name":"Edit","tool_input":{{"file_path":"{p}/src/sub.rs"}}"#
        ),
        true,
    );
    stop(&h, "alice", &p);
    let tasks = h.ok("bob", &["tasks", "lobby"]);
    assert!(!tasks.contains("src/sub.rs"), "{tasks}");
}

#[test]
fn a_path_outside_the_project_is_not_recorded() {
    let h = Home::new();
    let p = project(&h);
    h.ok("alice", &["join", "lobby"]);
    h.ok("alice", &["post", "lobby", "working: x"]);
    tool(
        &h,
        "alice",
        &p,
        r#""tool_name":"Edit","tool_input":{"file_path":"/etc/hosts"}"#,
        true,
    );
    stop(&h, "alice", &p);
    let tasks = h.ok("bob", &["tasks", "lobby"]);
    assert!(
        !tasks.contains("hosts") && !tasks.contains("/etc"),
        "{tasks}"
    );
}

/// The facts a successor needs from a session that ended: three files edited, a claim held and an ask still open.
/// What a successor can read through klatch afterwards, counted by fact. The measure behind #1368 and #1374, offline.
fn recoverable(with_machine_checkpoint: bool) -> usize {
    let h = Home::new();
    let p = project(&h);
    h.ok("alice", &["join", "lobby"]);
    h.ok("alice", &["post", "lobby", "working: parser rewrite"]);
    h.ok("alice", &["claim", "lobby", "docs/plan.md"]);
    h.ok("alice", &["ask", "lobby", "who-reviews-the-grammar"]);
    for f in ["src/a.rs", "src/b.rs", "src/c.rs"] {
        tool(
            &h,
            "alice",
            &p,
            &format!(r#""tool_name":"Edit","tool_input":{{"file_path":"{p}/{f}"}}"#),
            with_machine_checkpoint,
        );
    }
    let payload = format!(
        r#"{{"session_id":"s-alice","hook_event_name":"SessionEnd","cwd":"{p}","reason":"clear"}}"#
    );
    let mut args = vec!["hook", "--event", "SessionEnd"];
    if with_machine_checkpoint {
        args.push("--checkpoint");
    }
    hook_with_payload(&h, Some("alice"), &payload, &args);
    // everything the successor has: the tasks, the board, the claims and the log
    let seen = [
        h.ok("bob", &["tasks", "lobby"]),
        h.ok("bob", &["board", "lobby"]),
        h.ok("bob", &["claims", "lobby"]),
        h.ok("bob", &["read", "lobby", "--peek"]),
    ]
    .join("\n");
    [
        "src/a.rs",
        "src/b.rs",
        "src/c.rs",
        "docs/plan.md",
        "who-reviews-the-grammar",
    ]
    .iter()
    .filter(|fact| seen.contains(**fact))
    .count()
}

#[test]
fn a_successor_recovers_files_claims_and_open_asks_with_the_machine_checkpoint_and_only_the_ask_without()
 {
    let without = recoverable(false);
    let with = recoverable(true);
    println!("recoverable facts of 5: without {without}, with {with}");
    assert_eq!(
        without, 1,
        "only the ask is on the board without a checkpoint"
    );
    assert_eq!(with, 5);
}
