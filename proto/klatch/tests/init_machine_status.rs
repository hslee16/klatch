//! `klatch init --machine-status` (#1403): the consent for the machine-derived status lives in the hook commands.
//! Fixture settings files in a throwaway project and home only; nothing here touches a real settings file.
mod common;

use common::*;

const ARGS: [&str; 3] = ["--scope", "project", "--yes"];

fn with(extra: &[&'static str]) -> Vec<&'static str> {
    let mut v = ARGS.to_vec();
    v.extend_from_slice(extra);
    v
}

fn cmds(v: &serde_json::Value, event: &str) -> Vec<String> {
    v["hooks"][event]
        .as_array()
        .map_or_else(Vec::new, |_| hook_cmds(v, event))
}

fn has_flag(v: &serde_json::Value, event: &str, flag: &str) -> Vec<bool> {
    cmds(v, event)
        .iter()
        .map(|c| c.split(' ').any(|w| w == flag))
        .collect()
}

#[test]
fn the_machine_status_is_off_unless_asked_for() {
    let s = Setup::new();
    s.init(&ARGS, "");
    let v = Setup::read(&s.project_settings());
    assert!(cmds(&v, "Stop").is_empty());
    assert_eq!(has_flag(&v, "PostToolUse", "--machine-status"), vec![false]);
}

#[test]
fn machine_status_flags_post_tool_use_and_adds_a_stop_hook_and_nothing_else() {
    let s = Setup::new();
    let o = s.init(&with(&["--machine-status"]), "");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v = Setup::read(&s.project_settings());
    assert_eq!(has_flag(&v, "PostToolUse", "--machine-status"), vec![true]);
    let stop = cmds(&v, "Stop");
    assert_eq!(stop.len(), 1);
    assert!(
        stop[0].ends_with("--discover hook --event Stop --machine-status"),
        "{}",
        stop[0]
    );
    // SessionEnd needs no flag (it marks the session done as it always did); the other events are not flagged
    for event in ["SessionEnd", "SessionStart", "UserPromptSubmit"] {
        assert_eq!(
            has_flag(&v, event, "--machine-status"),
            vec![false],
            "{event}"
        );
    }
    // and the checkpoint consent is not given by it
    assert_eq!(has_flag(&v, "PostToolUse", "--checkpoint"), vec![false]);
}

#[test]
fn a_plain_rerun_keeps_the_consent_and_no_machine_status_removes_it() {
    let s = Setup::new();
    s.init(&with(&["--machine-status"]), "");
    let before = std::fs::read(s.project_settings()).unwrap();
    let o = s.init(&ARGS, "");
    assert!(out(&o).contains("already up to date"), "{}", out(&o));
    assert_eq!(std::fs::read(s.project_settings()).unwrap(), before);
    s.init(&with(&["--no-machine-status"]), "");
    let v = Setup::read(&s.project_settings());
    assert!(cmds(&v, "Stop").is_empty());
    assert_eq!(has_flag(&v, "PostToolUse", "--machine-status"), vec![false]);
}

#[test]
fn the_two_consents_share_one_stop_hook_and_each_can_be_removed_alone() {
    let s = Setup::new();
    s.init(&with(&["--checkpoint", "--machine-status"]), "");
    let v = Setup::read(&s.project_settings());
    let stop = cmds(&v, "Stop");
    assert_eq!(stop.len(), 1, "{stop:?}");
    assert!(
        stop[0].ends_with(" --checkpoint --machine-status"),
        "{stop:?}"
    );
    assert_eq!(has_flag(&v, "PostToolUse", "--checkpoint"), vec![true]);
    assert_eq!(has_flag(&v, "PostToolUse", "--machine-status"), vec![true]);
    // a plain rerun changes nothing
    let before = std::fs::read(s.project_settings()).unwrap();
    s.init(&ARGS, "");
    assert_eq!(std::fs::read(s.project_settings()).unwrap(), before);
    // the checkpoint goes: the machine status stays, with its Stop hook
    s.init(&with(&["--no-checkpoint"]), "");
    let v = Setup::read(&s.project_settings());
    let stop = cmds(&v, "Stop");
    assert_eq!(stop.len(), 1, "{stop:?}");
    assert!(
        stop[0].ends_with("--event Stop --machine-status"),
        "{stop:?}"
    );
    assert_eq!(has_flag(&v, "SessionEnd", "--checkpoint"), vec![false]);
    assert_eq!(has_flag(&v, "PostToolUse", "--machine-status"), vec![true]);
    // the other way round
    s.init(&with(&["--checkpoint", "--no-machine-status"]), "");
    let v = Setup::read(&s.project_settings());
    let stop = cmds(&v, "Stop");
    assert_eq!(stop.len(), 1, "{stop:?}");
    assert!(stop[0].ends_with("--event Stop --checkpoint"), "{stop:?}");
    assert_eq!(has_flag(&v, "PostToolUse", "--machine-status"), vec![false]);
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
    s.init(&with(&["--wake", "--machine-status"]), "");
    let v = Setup::read(&s.project_settings());
    assert_eq!(v["theme"], "dark");
    let stop = cmds(&v, "Stop");
    assert!(stop.iter().any(|c| c == "/usr/bin/audit"), "{stop:?}");
    assert!(
        stop.iter().any(|c| c.contains(" --discover wake ")),
        "{stop:?}"
    );
    assert!(
        stop.iter().any(|c| c.ends_with(" --machine-status")),
        "{stop:?}"
    );
    s.init(&with(&["--no-machine-status"]), "");
    let v = Setup::read(&s.project_settings());
    let stop = cmds(&v, "Stop");
    assert!(stop.iter().any(|c| c == "/usr/bin/audit"), "{stop:?}");
    assert!(
        stop.iter().any(|c| c.contains(" --discover wake ")),
        "{stop:?}"
    );
    assert!(
        !stop.iter().any(|c| c.contains("--machine-status")),
        "{stop:?}"
    );
}

#[test]
fn machine_status_needs_the_session_end_hook_and_uninstall_takes_everything_out() {
    let s = Setup::new();
    let o = s.init(&with(&["--machine-status", "--no-session-end"]), "");
    assert!(!o.status.success());
    assert!(!s.project_settings().exists(), "nothing was written");
    s.init(&with(&["--machine-status"]), "");
    s.init(&["--scope", "project", "--yes", "--uninstall"], "");
    assert!(
        !s.project_settings().exists()
            || cmds(&Setup::read(&s.project_settings()), "Stop").is_empty()
    );
}

#[test]
fn codex_refuses_it() {
    let s = Setup::new();
    let o = s.init(&["--codex", "--yes", "--machine-status"], "");
    assert!(!o.status.success());
}

#[test]
fn consent_shows_and_switches_the_machine_status() {
    let s = Setup::new();
    s.init(&ARGS, "");
    let shown = out(&s.klatch("consent", &["--scope", "project"], ""));
    assert!(shown.contains("machine-status: off"), "{shown}");
    s.klatch(
        "consent",
        &["machine-status", "on", "--scope", "project", "--yes"],
        "",
    );
    let v = Setup::read(&s.project_settings());
    assert_eq!(has_flag(&v, "PostToolUse", "--machine-status"), vec![true]);
    let shown = out(&s.klatch("consent", &["--scope", "project"], ""));
    assert!(shown.contains("machine-status: on"), "{shown}");
    assert!(shown.contains("checkpoint: off"), "{shown}");
    s.klatch(
        "consent",
        &["machine-status", "off", "--scope", "project", "--yes"],
        "",
    );
    let shown = out(&s.klatch("consent", &["--scope", "project"], ""));
    assert!(shown.contains("machine-status: off"), "{shown}");
}
