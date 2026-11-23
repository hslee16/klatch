//! The automatic machine checkpoint (#1408): the opt-in Stop and SessionEnd step writes a compact record of where a
//! member's session stands beside its durable task. Fixture hook inputs run through the real binary. The tool events
//! carry a secret-looking sentinel in the command and in the file content; it must never be stored or shown.
mod common;

use common::*;

const SECRET: &str = "SENTINEL-sk-live-4242";

/// One member's session in a project directory with a `.git` entry, so paths are taken relative to it.
struct Session<'a> {
    h: &'a Home,
    who: &'a str,
    proj: String,
}

impl<'a> Session<'a> {
    fn new(h: &'a Home, who: &'a str) -> Self {
        let p = h.0.join("proj");
        std::fs::create_dir_all(p.join(".git")).unwrap();
        Session {
            h,
            who,
            proj: p.to_str().unwrap().to_string(),
        }
    }

    /// Run the hook for `event` with `fields` after the common ones in the payload; the step must succeed.
    fn fire(&self, event: &str, fields: &str, flag: bool) {
        let (who, proj) = (self.who, &self.proj);
        let payload = format!(
            r#"{{"session_id":"s-{who}","hook_event_name":"{event}","cwd":"{proj}"{fields}}}"#
        );
        let o = self.run(event, &payload, flag);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }

    fn run(&self, event: &str, payload: &str, flag: bool) -> std::process::Output {
        let mut args = vec!["hook", "--event", event];
        if flag {
            args.push("--checkpoint");
        }
        hook_with_payload(self.h, Some(self.who), payload, &args)
    }

    /// A tool event whose body is the JSON fields after `cwd`.
    fn tool(&self, body: &str, flag: bool) {
        self.fire("PostToolUse", &format!(",{body}"), flag);
    }

    fn edit(&self, file: &str) {
        let proj = &self.proj;
        self.tool(
            &format!(
                r#""tool_name":"Edit","tool_input":{{"file_path":"{proj}/{file}","old_string":"{SECRET}","new_string":"{SECRET}"}},"tool_response":"{SECRET}""#
            ),
            true,
        );
    }

    /// An edit event of `file` with no secrets in it.
    fn plain_edit(&self, file: &str, flag: bool) {
        let proj = &self.proj;
        self.tool(
            &format!(r#""tool_name":"Edit","tool_input":{{"file_path":"{proj}/{file}"}}"#),
            flag,
        );
    }

    fn stop(&self) {
        self.fire("Stop", "", true);
    }

    fn end(&self, reason: &str) {
        self.fire("SessionEnd", &format!(r#","reason":"{reason}""#), true);
    }
}

/// The lines of a member in its home: join and a working line.
fn start(h: &Home, task: &str) {
    h.ok("alice", &["join", "lobby"]);
    h.ok("alice", &["post", "lobby", task]);
}

fn machine_part(tasks: &str) -> String {
    let i = tasks
        .find("machine checkpoint")
        .unwrap_or_else(|| panic!("no machine checkpoint in: {tasks}"));
    tasks[i..].to_string()
}

/// The text after the marker of the machine checkpoint in a `tasks` listing, without the closing bracket.
fn checkpoint_text(tasks: &str) -> String {
    let body = machine_part(tasks);
    let text = body.split("machine checkpoint").nth(1).unwrap();
    text.split_once(": ")
        .unwrap()
        .1
        .trim_end_matches(')')
        .trim()
        .to_string()
}

#[test]
fn edits_writes_and_commands_are_recorded_as_classes_and_paths_never_the_command_or_the_content() {
    let h = Home::new();
    let alice = Session::new(&h, "alice");
    start(&h, "working: parser rewrite");
    alice.edit("src/a.rs");
    let p = &alice.proj;
    alice.tool(
        &format!(
            r#""tool_name":"Write","tool_input":{{"file_path":"{p}/src/b.rs","content":"{SECRET}"}}"#
        ),
        true,
    );
    alice.tool(
        &format!(
            r#""tool_name":"Bash","tool_input":{{"command":"curl -H 'Authorization: {SECRET}' https://x"}}"#
        ),
        true,
    );
    alice.tool(
        &format!(r#""tool_name":"Read","tool_input":{{"file_path":"{p}/src/never-listed.rs"}}"#),
        true,
    );
    alice.stop();
    let tasks = h.ok("bob", &["tasks", "lobby"]);
    let m = machine_part(&tasks);
    assert_has(&m, &["src/a.rs", "src/b.rs", "tools edit write run read"]);
    // a read is a class only, and paths are repo-relative
    assert_lacks(&m, &["never-listed", p]);
    for shown in [
        tasks.as_str(),
        &h.ok("bob", &["read", "lobby", "--peek"]),
        &h.ok("bob", &["board", "lobby"]),
    ] {
        // a sentinel or a command line leaked
        assert_lacks(shown, &[SECRET, "curl"]);
    }
}

#[test]
fn nothing_is_recorded_unless_the_hook_carries_the_checkpoint_consent() {
    let h = Home::new();
    let alice = Session::new(&h, "alice");
    start(&h, "working: parser rewrite");
    alice.plain_edit("src/a.rs", false);
    let payload = format!(
        r#"{{"session_id":"s-alice","hook_event_name":"Stop","cwd":"{}"}}"#,
        alice.proj
    );
    alice.run("Stop", &payload, false);
    let tasks = h.ok("bob", &["tasks", "lobby"]);
    assert_lacks(&tasks, &["machine checkpoint"]);
}

#[test]
fn a_stop_writes_a_checkpoint_within_the_byte_cap_and_the_next_one_replaces_it() {
    let h = Home::new();
    let alice = Session::new(&h, "alice");
    start(&h, "working: big change");
    for i in 0..40 {
        alice.edit(&format!("crates/some/deep/module/path/file_{i:02}.rs"));
    }
    alice.stop();
    let text = checkpoint_text(&h.ok("bob", &["tasks", "lobby"]));
    assert!(text.len() <= 600, "{} bytes: {text}", text.len());
    assert_has(&text, &["file_39.rs"]);
    assert_lacks(&text, &["file_00.rs"]);
    alice.edit("src/later.rs");
    alice.stop();
    let second = h.ok("bob", &["tasks", "lobby"]);
    assert_eq!(second.matches("machine checkpoint").count(), 1, "{second}");
    assert_has(&second, &["src/later.rs"]);
}

#[test]
fn a_session_end_writes_the_final_checkpoint_with_the_claims_it_then_releases() {
    let h = Home::new();
    let alice = Session::new(&h, "alice");
    start(&h, "working: parser rewrite");
    h.ok("alice", &["claim", "lobby", "src/parser.rs"]);
    alice.edit("src/parser.rs");
    alice.end("clear");
    let claims = h.ok("bob", &["claims", "lobby"]);
    assert_lacks(&claims, &["src/parser.rs"]);
    let tasks = h.ok("bob", &["tasks", "lobby"]);
    let m = machine_part(&tasks);
    assert_has(
        &m,
        &[
            "session ended (clear)",
            "claims src/parser.rs",
            "files src/parser.rs",
        ],
    );
    // a reason outside the list is one word, never the text
    let h2 = Home::new();
    let other = Session::new(&h2, "alice");
    start(&h2, "working: x");
    other.end(&format!("weird {SECRET}"));
    let t2 = h2.ok("bob", &["tasks", "lobby"]);
    assert_has(&t2, &["session ended (other)"]);
    assert_lacks(&t2, &[SECRET]);
}

#[test]
fn an_explicit_checkpoint_wins_and_the_machine_one_is_marked_beneath_it() {
    let h = Home::new();
    let alice = Session::new(&h, "alice");
    start(&h, "working: parser rewrite");
    h.ok(
        "alice",
        &[
            "post",
            "lobby",
            "checkpoint: tokens done, resume at the grammar",
        ],
    );
    alice.edit("src/a.rs");
    alice.stop();
    let tasks = h.ok("bob", &["tasks", "lobby"]);
    let line = tasks.lines().find(|l| l.starts_with("alice:")).unwrap();
    let explicit = line.find("resume at the grammar").unwrap();
    let machine = line.find("machine checkpoint").unwrap();
    assert!(
        explicit < machine,
        "the explicit checkpoint comes first: {line}"
    );
    assert_has(line, &["src/a.rs"]);
}

#[test]
fn a_member_with_no_task_still_leaves_its_checkpoint_for_a_successor() {
    let h = Home::new();
    let alice = Session::new(&h, "alice");
    h.ok("alice", &["join", "lobby"]);
    alice.edit("src/a.rs");
    alice.end("other");
    let tasks = h.ok("bob", &["tasks", "lobby"]);
    // it says no task was declared
    assert_has(&tasks, &["alice", "src/a.rs", "no task"]);
}

#[test]
fn a_tool_call_made_inside_a_subagent_is_not_the_parents_work() {
    let h = Home::new();
    let alice = Session::new(&h, "alice");
    start(&h, "working: x");
    let p = &alice.proj;
    alice.tool(
        &format!(
            r#""agent_id":"sub-1","tool_name":"Edit","tool_input":{{"file_path":"{p}/src/sub.rs"}}"#
        ),
        true,
    );
    alice.stop();
    let tasks = h.ok("bob", &["tasks", "lobby"]);
    assert_lacks(&tasks, &["src/sub.rs"]);
}

#[test]
fn a_path_outside_the_project_is_not_recorded() {
    let h = Home::new();
    let alice = Session::new(&h, "alice");
    start(&h, "working: x");
    alice.tool(
        r#""tool_name":"Edit","tool_input":{"file_path":"/etc/hosts"}"#,
        true,
    );
    alice.stop();
    let tasks = h.ok("bob", &["tasks", "lobby"]);
    assert_lacks(&tasks, &["hosts", "/etc"]);
}

/// The facts a successor needs from a session that ended: three files edited, a claim held and an ask still open.
/// What a successor can read through klatch afterwards, counted by fact. The measure behind #1368 and #1374, offline.
fn recoverable(with_machine_checkpoint: bool) -> usize {
    let h = Home::new();
    let alice = Session::new(&h, "alice");
    start(&h, "working: parser rewrite");
    h.ok("alice", &["claim", "lobby", "docs/plan.md"]);
    h.ok("alice", &["ask", "lobby", "who-reviews-the-grammar"]);
    for f in ["src/a.rs", "src/b.rs", "src/c.rs"] {
        alice.plain_edit(f, with_machine_checkpoint);
    }
    let payload = format!(
        r#"{{"session_id":"s-alice","hook_event_name":"SessionEnd","cwd":"{}","reason":"clear"}}"#,
        alice.proj
    );
    alice.run("SessionEnd", &payload, with_machine_checkpoint);
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

#[test]
fn the_session_start_briefing_of_a_successor_shows_the_machine_checkpoint() {
    let h = Home::new();
    let alice = Session::new(&h, "alice");
    start(&h, "working: parser rewrite");
    h.ok("alice", &["claim", "lobby", "docs/plan.md"]);
    alice.edit("src/a.rs");
    alice.end("clear");
    let o = hook_with_payload(
        &h,
        Some("bob"),
        r#"{"session_id":"s-bob","source":"startup"}"#,
        &["hook", "--event", "SessionStart"],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let text = String::from_utf8_lossy(&o.stdout).to_string();
    assert_has(&text, &["machine checkpoint", "claims docs/plan.md"]);
    assert_lacks(&text, &[SECRET]);
}
