//! `klatch init --passive-claims` (#1401): the consent for passive claims lives in the hook commands. Fixture settings
//! files in a throwaway project and home only; nothing here touches a real settings file.
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

fn has_flag(v: &serde_json::Value, event: &str, flag: &str) -> Vec<bool> {
    cmds(v, event)
        .iter()
        .map(|c| c.split(' ').any(|w| w == flag))
        .collect()
}

fn passive(v: &serde_json::Value, event: &str) -> Vec<bool> {
    has_flag(v, event, "--passive-claims")
}

#[test]
fn passive_claims_are_off_unless_asked_for() {
    let s = Setup::new();
    s.init(&ARGS, "");
    let v = Setup::read(&s.project_settings());
    assert!(cmds(&v, "Stop").is_empty());
    assert_eq!(passive(&v, "PostToolUse"), vec![false]);
    assert_eq!(passive(&v, "SessionEnd"), vec![false]);
}

#[test]
fn passive_claims_flag_post_tool_use_and_session_end_and_add_a_stop_hook() {
    let s = Setup::new();
    let o = s.init(&with(&["--passive-claims"]), "");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v = Setup::read(&s.project_settings());
    assert_eq!(passive(&v, "PostToolUse"), vec![true]);
    assert_eq!(passive(&v, "SessionEnd"), vec![true]);
    let stop = cmds(&v, "Stop");
    assert_eq!(stop.len(), 1);
    assert!(
        stop[0].ends_with("--discover hook --event Stop --passive-claims"),
        "{}",
        stop[0]
    );
    assert_eq!(passive(&v, "SessionStart"), vec![false]);
    assert_eq!(passive(&v, "UserPromptSubmit"), vec![false]);
    // the checkpoint consent is not implied
    assert_eq!(has_flag(&v, "PostToolUse", "--checkpoint"), vec![false]);
}

#[test]
fn a_plain_rerun_keeps_the_consent_and_no_passive_claims_removes_it() {
    let s = Setup::new();
    s.init(&with(&["--passive-claims"]), "");
    let before = std::fs::read(s.project_settings()).unwrap();
    let o = s.init(&ARGS, "");
    assert!(out(&o).contains("already up to date"), "{}", out(&o));
    assert_eq!(std::fs::read(s.project_settings()).unwrap(), before);
    s.init(&with(&["--no-passive-claims"]), "");
    let v = Setup::read(&s.project_settings());
    assert!(cmds(&v, "Stop").is_empty());
    assert_eq!(passive(&v, "PostToolUse"), vec![false]);
    assert_eq!(passive(&v, "SessionEnd"), vec![false]);
}

#[test]
fn both_consents_share_one_stop_hook_and_each_comes_off_alone() {
    let s = Setup::new();
    s.init(&with(&["--checkpoint", "--passive-claims"]), "");
    let v = Setup::read(&s.project_settings());
    let stop = cmds(&v, "Stop");
    assert_eq!(stop.len(), 1, "{stop:?}");
    assert!(
        stop[0].ends_with(" --passive-claims --checkpoint"),
        "{}",
        stop[0]
    );
    assert!(cmds(&v, "PostToolUse")[0].ends_with(" --passive-claims --checkpoint"));
    s.init(&with(&["--no-passive-claims"]), "");
    let v = Setup::read(&s.project_settings());
    let stop = cmds(&v, "Stop");
    assert_eq!(stop.len(), 1, "{stop:?}");
    assert!(
        stop[0].ends_with("--event Stop --checkpoint"),
        "{}",
        stop[0]
    );
    assert_eq!(passive(&v, "PostToolUse"), vec![false]);
    assert_eq!(has_flag(&v, "PostToolUse", "--checkpoint"), vec![true]);
    s.init(&with(&["--passive-claims", "--no-checkpoint"]), "");
    let v = Setup::read(&s.project_settings());
    assert!(cmds(&v, "Stop")[0].ends_with("--event Stop --passive-claims"));
    assert_eq!(has_flag(&v, "SessionEnd", "--checkpoint"), vec![false]);
    assert_eq!(passive(&v, "SessionEnd"), vec![true]);
}

#[test]
fn it_leaves_the_wake_watcher_and_other_hooks_alone_and_uninstall_takes_everything_out() {
    let s = Setup::new();
    std::fs::create_dir_all(s.project().join(".claude")).unwrap();
    std::fs::write(
        s.project_settings(),
        r#"{"theme":"dark","hooks":{"Stop":[{"hooks":[{"type":"command","command":"/usr/bin/audit"}]}]}}"#,
    )
    .unwrap();
    s.init(&with(&["--wake", "--passive-claims"]), "");
    let v = Setup::read(&s.project_settings());
    assert_eq!(v["theme"], "dark");
    let stop = cmds(&v, "Stop");
    assert!(stop.iter().any(|c| c == "/usr/bin/audit"), "{stop:?}");
    assert!(
        stop.iter().any(|c| c.contains(" --discover wake ")),
        "{stop:?}"
    );
    assert!(
        stop.iter().any(|c| c.ends_with(" --passive-claims")),
        "{stop:?}"
    );
    s.init(&with(&["--no-passive-claims"]), "");
    let stop = cmds(&Setup::read(&s.project_settings()), "Stop");
    assert!(stop.iter().any(|c| c == "/usr/bin/audit"), "{stop:?}");
    assert!(
        stop.iter().any(|c| c.contains(" --discover wake ")),
        "{stop:?}"
    );
    assert!(
        !stop.iter().any(|c| c.contains("--passive-claims")),
        "{stop:?}"
    );
    s.init(&with(&["--passive-claims"]), "");
    s.init(&["--scope", "project", "--yes", "--uninstall"], "");
    let v = Setup::read(&s.project_settings());
    assert!(
        !cmds(&v, "Stop").iter().any(|c| c.contains("klatch")),
        "{:?}",
        cmds(&v, "Stop")
    );
    assert_eq!(cmds(&v, "Stop"), vec!["/usr/bin/audit".to_string()]);
}

#[test]
fn passive_claims_do_not_need_the_session_end_hook_and_codex_refuses_them() {
    let s = Setup::new();
    let o = s.init(&with(&["--passive-claims", "--no-session-end"]), "");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v = Setup::read(&s.project_settings());
    assert_eq!(passive(&v, "PostToolUse"), vec![true]);
    assert!(cmds(&v, "SessionEnd").is_empty());
    let codex = Setup::new();
    let o = codex.init(&["--codex", "--yes", "--passive-claims"], "");
    assert!(!o.status.success());
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("--passive-claims"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn consent_shows_and_switches_passive_claims() {
    let s = Setup::new();
    s.init(&ARGS, "");
    let shown = out(&s.klatch("consent", &["--scope", "project"], ""));
    assert!(shown.contains("passive-claims: off"), "{shown}");
    s.klatch(
        "consent",
        &["passive-claims", "on", "--scope", "project", "--yes"],
        "",
    );
    let v = Setup::read(&s.project_settings());
    assert_eq!(passive(&v, "PostToolUse"), vec![true]);
    let shown = out(&s.klatch("consent", &["--scope", "project"], ""));
    assert!(shown.contains("passive-claims: on"), "{shown}");
    assert!(shown.contains("checkpoint: off"), "{shown}");
    s.klatch(
        "consent",
        &["passive-claims", "off", "--scope", "project", "--yes"],
        "",
    );
    let shown = out(&s.klatch("consent", &["--scope", "project"], ""));
    assert!(shown.contains("passive-claims: off"), "{shown}");
    assert!(cmds(&Setup::read(&s.project_settings()), "Stop").is_empty());
}

#[test]
fn switching_session_end_off_and_on_keeps_the_consents() {
    let s = Setup::new();
    s.init(&with(&["--checkpoint", "--passive-claims"]), "");
    s.klatch(
        "consent",
        &["session-end", "off", "--scope", "project", "--yes"],
        "",
    );
    assert!(cmds(&Setup::read(&s.project_settings()), "SessionEnd").is_empty());
    s.klatch(
        "consent",
        &["session-end", "on", "--scope", "project", "--yes"],
        "",
    );
    let v = Setup::read(&s.project_settings());
    assert_eq!(passive(&v, "SessionEnd"), vec![true]);
    assert_eq!(has_flag(&v, "SessionEnd", "--checkpoint"), vec![true]);
}
