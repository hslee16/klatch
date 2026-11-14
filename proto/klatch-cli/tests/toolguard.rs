//! The PreToolUse claim hook (#33): an edit of a path another member claimed is blocked with exit 2, a command that
//! only mentions one is warned about, and a broken gateway fails open with a visible line.
mod common;

use common::*;

fn call(tool: &str, input: serde_json::Value) -> String {
    serde_json::json!({
        "session_id": "s-1",
        "hook_event_name": "PreToolUse",
        "tool_name": tool,
        "tool_input": input,
        "cwd": "/w",
    })
    .to_string()
}

fn pre(h: &Home, who: &str, stdin: &str) -> std::process::Output {
    h.run_stdin(who, &["hook", "--event", "PreToolUse"], stdin)
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// bob claimed src/api.rs; alice is in the room.
fn claimed() -> Home {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.ok("bob", &["join", "r"]);
    h.ok(
        "bob",
        &["claim", "r", "/w/src/api.rs", "--reason", "refactor"],
    );
    h
}

#[test]
fn an_edit_of_a_claimed_path_is_blocked_with_exit_two_and_the_reason_on_stderr() {
    let h = claimed();
    let o = pre(
        &h,
        "alice",
        &call("Edit", serde_json::json!({"file_path": "/w/src/api.rs"})),
    );
    assert_eq!(o.status.code(), Some(2), "{}", text(&o.stderr));
    let err = text(&o.stderr);
    assert!(
        err.contains("bob") && err.contains("/w/src/api.rs"),
        "{err}"
    );
    assert!(err.contains("Do not edit it"), "{err}");
    assert!(o.stdout.is_empty());
}

#[test]
fn every_edit_tool_is_judged() {
    let h = claimed();
    for (tool, input) in [
        (
            "Write",
            serde_json::json!({"file_path": "/w/src/api.rs", "content": "x"}),
        ),
        (
            "MultiEdit",
            serde_json::json!({"file_path": "/w/src/api.rs", "edits": []}),
        ),
        (
            "NotebookEdit",
            serde_json::json!({"notebook_path": "/w/src/api.rs"}),
        ),
        (
            "apply_patch",
            serde_json::json!({"command": "*** Begin Patch\n*** Update File: /w/src/api.rs\n*** End Patch"}),
        ),
    ] {
        let o = pre(&h, "alice", &call(tool, input));
        assert_eq!(o.status.code(), Some(2), "{tool}: {}", text(&o.stderr));
    }
}

#[test]
fn an_edit_of_an_unclaimed_path_or_of_your_own_claim_is_allowed_silently() {
    let h = claimed();
    for (who, file) in [("alice", "/w/src/other.rs"), ("bob", "/w/src/api.rs")] {
        let o = pre(
            &h,
            who,
            &call("Edit", serde_json::json!({"file_path": file})),
        );
        assert_eq!(o.status.code(), Some(0), "{who} {file}");
        assert!(o.stdout.is_empty() && o.stderr.is_empty(), "{who} {file}");
    }
}

#[test]
fn a_shell_command_that_mentions_a_claimed_path_is_allowed_with_a_warning_in_the_context() {
    let h = claimed();
    let o = pre(
        &h,
        "alice",
        &call(
            "Bash",
            serde_json::json!({"command": "sed -i s/a/b/ /w/src/api.rs"}),
        ),
    );
    assert_eq!(o.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(text(&o.stdout).trim()).unwrap();
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    let ctx = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(ctx.contains("bob") && ctx.contains("claimed path"), "{ctx}");
    let quiet = pre(
        &h,
        "alice",
        &call("Bash", serde_json::json!({"command": "cargo test"})),
    );
    assert!(quiet.stdout.is_empty() && quiet.status.success());
}

#[test]
fn a_released_claim_no_longer_blocks() {
    let h = claimed();
    h.ok("bob", &["release", "r", "/w/src/api.rs"]);
    let o = pre(
        &h,
        "alice",
        &call("Edit", serde_json::json!({"file_path": "/w/src/api.rs"})),
    );
    assert_eq!(o.status.code(), Some(0));
}

#[test]
fn a_hook_checking_claims_never_consumes_messages() {
    let h = claimed();
    h.ok("bob", &["post", "r", "hello alice"]);
    pre(
        &h,
        "alice",
        &call("Edit", serde_json::json!({"file_path": "/w/src/api.rs"})),
    );
    assert!(h.ok("alice", &["read", "r"]).contains("hello alice"));
}

#[test]
fn input_that_is_not_a_tool_call_is_allowed_and_never_panics() {
    let h = claimed();
    for stdin in [
        "",
        "not json",
        "{}",
        "[1]",
        r#"{"tool_name":7,"tool_input":"x"}"#,
        r#"{"tool_name":"Edit"}"#,
    ] {
        let o = pre(&h, "alice", stdin);
        assert_eq!(o.status.code(), Some(0), "{stdin:?}: {}", text(&o.stderr));
    }
}

#[test]
fn a_broken_store_fails_open_with_a_visible_line_and_never_blocks() {
    let h = claimed();
    std::fs::write(h.0.join("rooms.db"), b"garbage garbage garbage").unwrap();
    let o = pre(
        &h,
        "alice",
        &call("Edit", serde_json::json!({"file_path": "/w/src/api.rs"})),
    );
    assert_eq!(o.status.code(), Some(0));
    assert!(
        text(&o.stdout).contains("could not check claims"),
        "{}",
        text(&o.stdout)
    );
}

/// A real directory `real` and a symlink `link` to it, under a fresh temp dir.
fn linked() -> (std::path::PathBuf, std::path::PathBuf) {
    let base = std::env::temp_dir().join(format!(
        "klatch-links-{}-{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let base = base.canonicalize().unwrap_or(base);
    std::fs::create_dir_all(base.join("real")).unwrap();
    std::os::unix::fs::symlink(base.join("real"), base.join("link")).unwrap();
    (base.join("real"), base.join("link"))
}

fn edit_of(h: &Home, who: &str, file: &std::path::Path) -> Option<i32> {
    pre(
        h,
        who,
        &call("Edit", serde_json::json!({"file_path": file})),
    )
    .status
    .code()
}

#[test]
fn an_edit_through_a_symlink_to_a_claimed_file_is_blocked() {
    let (real, link) = linked();
    std::fs::write(real.join("api.rs"), "x").unwrap();
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.ok("bob", &["join", "r"]);
    h.ok(
        "bob",
        &["claim", "r", real.join("api.rs").to_str().unwrap()],
    );
    assert_eq!(edit_of(&h, "alice", &link.join("api.rs")), Some(2));
    assert_eq!(edit_of(&h, "alice", &real.join("other.rs")), Some(0));
}

#[test]
fn a_claim_made_through_a_symlink_blocks_an_edit_by_the_real_path() {
    let (real, link) = linked();
    std::fs::write(real.join("api.rs"), "x").unwrap();
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.ok("bob", &["join", "r"]);
    h.ok(
        "bob",
        &["claim", "r", link.join("api.rs").to_str().unwrap()],
    );
    assert_eq!(edit_of(&h, "alice", &real.join("api.rs")), Some(2));
}

#[test]
fn a_file_that_does_not_exist_yet_under_a_symlinked_directory_is_judged_by_where_it_will_land() {
    let (real, link) = linked();
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.ok("bob", &["join", "r"]);
    h.ok(
        "bob",
        &["claim", "r", real.join("new.rs").to_str().unwrap()],
    );
    assert_eq!(edit_of(&h, "alice", &link.join("new.rs")), Some(2));
}

#[test]
fn a_dot_dot_spelling_of_a_claimed_path_is_blocked() {
    let h = claimed();
    let o = pre(
        &h,
        "alice",
        &call(
            "Edit",
            serde_json::json!({"file_path": "/w/x/../src/./api.rs"}),
        ),
    );
    assert_eq!(o.status.code(), Some(2));
}
