//! `klatch init --checkpoint` (#1408): the consent for the machine checkpoint lives in the hook commands. Fixture
//! settings files in a throwaway project and home only; nothing here touches a real settings file.
mod common;

use common::*;

const ARGS: [&str; 3] = ["--scope", "project", "--yes"];

fn with(extra: &[&'static str]) -> Vec<&'static str> {
    let mut v = ARGS.to_vec();
    v.extend_from_slice(extra);
    v
}

/// The commands of an event, none when the event is not there.
fn cmds(v: &serde_json::Value, event: &str) -> Vec<String> {
    v["hooks"][event]
        .as_array()
        .map_or_else(Vec::new, |_| hook_cmds(v, event))
}

fn flagged(v: &serde_json::Value, event: &str) -> Vec<bool> {
    cmds(v, event)
        .iter()
        .map(|c| c.ends_with(" --checkpoint"))
        .collect()
}

#[test]
fn the_checkpoint_is_off_unless_asked_for() {
    let s = Setup::new();
    s.init(&ARGS, "");
    let v = Setup::read(&s.project_settings());
    assert!(cmds(&v, "Stop").is_empty());
    assert_eq!(flagged(&v, "PostToolUse"), vec![false]);
    assert_eq!(flagged(&v, "SessionEnd"), vec![false]);
}

#[test]
fn checkpoint_adds_a_stop_hook_and_flags_post_tool_use_and_session_end() {
    let s = Setup::new();
    let o = s.init(&with(&["--checkpoint"]), "");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v = Setup::read(&s.project_settings());
    assert_eq!(flagged(&v, "PostToolUse"), vec![true]);
    assert_eq!(flagged(&v, "SessionEnd"), vec![true]);
    let stop = cmds(&v, "Stop");
    assert_eq!(stop.len(), 1);
    assert!(
        stop[0].ends_with("--discover hook --event Stop --checkpoint"),
        "{}",
        stop[0]
    );
    // the other events are not flagged
    assert_eq!(flagged(&v, "SessionStart"), vec![false]);
    assert_eq!(flagged(&v, "UserPromptSubmit"), vec![false]);
}

#[test]
fn a_plain_rerun_keeps_the_consent_and_no_checkpoint_removes_it() {
    let s = Setup::new();
    s.init(&with(&["--checkpoint"]), "");
    let before = std::fs::read(s.project_settings()).unwrap();
    let o = s.init(&ARGS, "");
    assert!(out(&o).contains("already up to date"), "{}", out(&o));
    assert_eq!(std::fs::read(s.project_settings()).unwrap(), before);
    s.init(&with(&["--no-checkpoint"]), "");
    let v = Setup::read(&s.project_settings());
    assert!(cmds(&v, "Stop").is_empty());
    assert_eq!(flagged(&v, "PostToolUse"), vec![false]);
    assert_eq!(flagged(&v, "SessionEnd"), vec![false]);
}

#[test]
fn it_leaves_the_wake_watcher_and_other_hooks_alone() {
    let s = Setup::new();
    std::fs::create_dir_all(s.project().join(".claude")).unwrap();
    std::fs::write(
        s.project_settings(),
        r#"{"theme":"dark","hooks":{"Stop":[{"hooks":[{"type":"command","command":"/usr/bin/audit"}]}]}}"#,
    )
    .unwrap();
    s.init(&with(&["--wake", "--checkpoint"]), "");
    let v = Setup::read(&s.project_settings());
    assert_eq!(v["theme"], "dark");
    let stop = cmds(&v, "Stop");
    assert!(stop.iter().any(|c| c == "/usr/bin/audit"), "{stop:?}");
    assert!(
        stop.iter().any(|c| c.contains(" --discover wake ")),
        "{stop:?}"
    );
    assert!(
        stop.iter().any(|c| c.ends_with(" --checkpoint")),
        "{stop:?}"
    );
    s.init(&with(&["--no-checkpoint"]), "");
    let v = Setup::read(&s.project_settings());
    let stop = cmds(&v, "Stop");
    assert!(stop.iter().any(|c| c == "/usr/bin/audit"), "{stop:?}");
    assert!(
        stop.iter().any(|c| c.contains(" --discover wake ")),
        "{stop:?}"
    );
    assert!(
        !stop.iter().any(|c| c.ends_with(" --checkpoint")),
        "{stop:?}"
    );
}

#[test]
fn checkpoint_needs_the_session_end_hook_and_uninstall_takes_everything_out() {
    let s = Setup::new();
    let o = s.init(&with(&["--checkpoint", "--no-session-end"]), "");
    assert!(!o.status.success());
    assert!(!s.project_settings().exists(), "nothing was written");
    s.init(&with(&["--checkpoint"]), "");
    s.init(&["--scope", "project", "--yes", "--uninstall"], "");
    assert!(
        !s.project_settings().exists()
            || cmds(&Setup::read(&s.project_settings()), "Stop").is_empty()
    );
}

#[test]
fn consent_shows_and_switches_the_checkpoint() {
    let s = Setup::new();
    s.init(&ARGS, "");
    let shown = out(&s.klatch("consent", &["--scope", "project"], ""));
    assert!(shown.contains("checkpoint: off"), "{shown}");
    s.klatch(
        "consent",
        &["checkpoint", "on", "--scope", "project", "--yes"],
        "",
    );
    let v = Setup::read(&s.project_settings());
    assert_eq!(flagged(&v, "PostToolUse"), vec![true]);
    let shown = out(&s.klatch("consent", &["--scope", "project"], ""));
    assert!(shown.contains("checkpoint: on"), "{shown}");
    s.klatch(
        "consent",
        &["checkpoint", "off", "--scope", "project", "--yes"],
        "",
    );
    let shown = out(&s.klatch("consent", &["--scope", "project"], ""));
    assert!(shown.contains("checkpoint: off"), "{shown}");
}
