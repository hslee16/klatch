//! End-to-end tests: run the real binary against a throwaway KLATCH_HOME.
mod common;

use common::*;

#[test]
fn init_project_yes_writes_the_hooks_with_the_absolute_binary_path_and_nothing_else() {
    let s = Setup::new();
    let o = s.init(&["--scope", "project", "--yes"], "");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v = Setup::read(&s.project_settings());
    let bin = env!("CARGO_BIN_EXE_klatch");
    for e in ["SessionStart", "UserPromptSubmit", "PostToolUse"] {
        let c = hook_cmds(&v, e);
        assert_eq!(c.len(), 1, "{e}");
        assert!(
            c[0].contains(bin) && c[0].contains("--discover hook --event"),
            "{}",
            c[0]
        );
        assert!(!c[0].contains('$'), "a shell variable in {}", c[0]);
    }
    assert!(!hook_cmds(&v, "SessionStart")[0].contains("--board"));
    assert!(
        !s.user_settings().exists(),
        "user scope written without being asked"
    );
}

#[test]
fn init_is_safe_to_run_again() {
    let s = Setup::new();
    s.init(&["--scope", "project", "--yes"], "");
    let before = std::fs::read(s.project_settings()).unwrap();
    let o = s.init(&["--scope", "project", "--yes"], "");
    assert!(out(&o).contains("already up to date"), "got: {}", out(&o));
    assert_eq!(std::fs::read(s.project_settings()).unwrap(), before);
}

#[test]
fn init_user_and_both_write_the_scopes_asked_for() {
    let s = Setup::new();
    s.init(&["--scope", "user", "--yes"], "");
    assert!(s.user_settings().exists() && !s.project_settings().exists());
    let b = Setup::new();
    b.init(&["--scope", "both", "--yes"], "");
    assert!(b.user_settings().exists() && b.project_settings().exists());
}

#[test]
fn init_yes_without_a_scope_refuses_and_writes_nothing() {
    let s = Setup::new();
    let o = s.init(&["--yes"], "");
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("--scope"));
    assert!(!s.project_settings().exists() && !s.user_settings().exists());
}

#[test]
fn init_asks_before_writing_shows_the_diff_and_a_no_writes_nothing() {
    let s = Setup::new();
    let o = s.init(&["--scope", "project"], "n\ny\ny\nn\n");
    assert!(o.status.success());
    assert!(
        out(&o).contains("+ ") && out(&o).contains("--discover hook --event"),
        "no diff: {}",
        out(&o)
    );
    assert!(!s.project_settings().exists(), "wrote without a yes");
    let y = s.init(&["--scope", "project"], "n\ny\ny\ny\n");
    assert!(y.status.success());
    assert!(s.project_settings().exists());
}

#[test]
fn init_both_consents_to_each_scope_separately() {
    let s = Setup::new();
    s.init(&["--scope", "both"], "n\ny\ny\ny\nn\n");
    assert!(s.project_settings().exists(), "project was said yes");
    assert!(!s.user_settings().exists(), "user was said no");
}

#[test]
fn the_board_is_off_unless_asked_for_and_only_on_session_start() {
    let s = Setup::new();
    s.init(&["--scope", "project", "--yes"], "");
    assert!(!hook_cmds(&Setup::read(&s.project_settings()), "SessionStart")[0].contains("--board"));
    let b = Setup::new();
    b.init(&["--scope", "project", "--yes", "--board", "lobby"], "");
    let v = Setup::read(&b.project_settings());
    assert!(hook_cmds(&v, "SessionStart")[0].ends_with("--board lobby"));
    assert!(!hook_cmds(&v, "PostToolUse")[0].contains("--board"));
    let i = Setup::new();
    i.init(&["--scope", "project"], "y\ny\ny\ny\n");
    assert!(
        hook_cmds(&Setup::read(&i.project_settings()), "SessionStart")[0].contains("--board lobby")
    );
}

#[test]
fn init_keeps_existing_settings_and_their_order() {
    let s = Setup::new();
    std::fs::create_dir_all(s.project().join(".claude")).unwrap();
    std::fs::write(
        s.project_settings(),
        r#"{"zeta":1,"permissions":{"allow":["Bash(ls)"]},"hooks":{"PostToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"/usr/bin/audit"}]}]}}"#,
    )
    .unwrap();
    s.init(&["--scope", "project", "--yes"], "");
    let text = std::fs::read_to_string(s.project_settings()).unwrap();
    assert!(
        text.find("zeta").unwrap() < text.find("permissions").unwrap(),
        "keys were reordered"
    );
    let v = Setup::read(&s.project_settings());
    assert_eq!(v["permissions"]["allow"][0], "Bash(ls)");
    assert!(hook_cmds(&v, "PostToolUse").contains(&"/usr/bin/audit".to_string()));
}

#[test]
fn init_refuses_an_unreadable_settings_file_and_writes_no_scope_at_all() {
    let s = Setup::new();
    std::fs::create_dir_all(s.home().join(".claude")).unwrap();
    std::fs::write(s.user_settings(), "{ not json").unwrap();
    let o = s.init(&["--scope", "both", "--yes"], "");
    assert!(!o.status.success());
    assert_eq!(
        std::fs::read_to_string(s.user_settings()).unwrap(),
        "{ not json"
    );
    assert!(
        !s.project_settings().exists(),
        "wrote one scope although the other was unreadable"
    );
}

#[test]
fn hooks_shows_when_each_event_last_ran_with_a_count_and_no_content() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.ok("bob", &["post", "r", "secret-looking message body"]);
    assert!(
        h.ok("alice", &["hooks"]).contains("no hook has run"),
        "before any run"
    );
    h.ok("alice", &["hook", "--event", "SessionStart"]);
    h.ok("alice", &["hook", "--event", "PostToolUse"]);
    h.ok("alice", &["hook", "--event", "PostToolUse"]);
    let own = h.ok("alice", &["hooks"]);
    assert!(
        own.contains("SessionStart") && own.contains("runs 1"),
        "got: {own}"
    );
    assert!(
        own.contains("PostToolUse") && own.contains("runs 2"),
        "got: {own}"
    );
    assert!(
        !own.contains("secret-looking"),
        "message content leaked: {own}"
    );
    let other = h.ok("bob", &["hooks", "--of", "alice"]);
    assert!(
        other.contains("PostToolUse") && other.contains("runs 2"),
        "got: {other}"
    );
}
