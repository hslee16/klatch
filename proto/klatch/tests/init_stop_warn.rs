//! `klatch init --stop-warn` (#1344): the consent for the stop warning lives in the hook commands.
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
fn the_stop_warning_is_off_unless_asked_for() {
    let s = Setup::new();
    s.init(&ARGS, "");
    let v = Setup::read(&s.project_settings());
    assert!(cmds(&v, "Stop").is_empty());
    assert_eq!(has_flag(&v, "PostToolUse", "--stop-warn"), vec![false]);
}

#[test]
fn stop_warn_flags_post_tool_use_and_session_end_adds_the_stop_hook_and_nothing_else() {
    let s = Setup::new();
    let o = s.init(&with(&["--stop-warn"]), "");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v = Setup::read(&s.project_settings());
    assert_eq!(has_flag(&v, "PostToolUse", "--stop-warn"), vec![true]);
    assert_eq!(has_flag(&v, "SessionEnd", "--stop-warn"), vec![true]);
    let stop = cmds(&v, "Stop");
    assert_eq!(stop.len(), 1, "{stop:?}");
    assert!(
        stop[0].ends_with("--discover hook --event Stop --stop-warn"),
        "{}",
        stop[0]
    );
    for event in ["SessionStart", "UserPromptSubmit"] {
        assert_eq!(has_flag(&v, event, "--stop-warn"), vec![false], "{event}");
    }
    // no other consent is given by it, and no SubagentStop hook is registered
    for flag in ["--checkpoint", "--machine-status", "--passive-claims"] {
        assert_eq!(has_flag(&v, "PostToolUse", flag), vec![false], "{flag}");
    }
    assert!(v["hooks"].get("SubagentStop").is_none());
}

#[test]
fn a_plain_rerun_keeps_the_consent_and_no_stop_warn_removes_it() {
    let s = Setup::new();
    s.init(&with(&["--stop-warn"]), "");
    let before = std::fs::read(s.project_settings()).unwrap();
    let o = s.init(&ARGS, "");
    assert!(out(&o).contains("already up to date"), "{}", out(&o));
    assert_eq!(std::fs::read(s.project_settings()).unwrap(), before);
    s.init(&with(&["--no-stop-warn"]), "");
    let v = Setup::read(&s.project_settings());
    assert!(cmds(&v, "Stop").is_empty());
    assert_eq!(has_flag(&v, "PostToolUse", "--stop-warn"), vec![false]);
}

#[test]
fn it_shares_one_stop_hook_in_the_fixed_flag_order_and_each_can_be_removed_alone() {
    let s = Setup::new();
    s.init(&with(&["--machine-status", "--stop-warn"]), "");
    let v = Setup::read(&s.project_settings());
    let stop = cmds(&v, "Stop");
    assert_eq!(stop.len(), 1, "{stop:?}");
    assert!(
        stop[0].ends_with(" --machine-status --stop-warn"),
        "{stop:?}"
    );
    s.init(&with(&["--no-machine-status"]), "");
    let stop = cmds(&Setup::read(&s.project_settings()), "Stop");
    assert!(stop[0].ends_with("--event Stop --stop-warn"), "{stop:?}");
    s.init(&with(&["--machine-status", "--no-stop-warn"]), "");
    let stop = cmds(&Setup::read(&s.project_settings()), "Stop");
    assert!(
        stop[0].ends_with("--event Stop --machine-status"),
        "{stop:?}"
    );
}

#[test]
fn it_leaves_other_hooks_alone_and_uninstall_takes_it_out() {
    let s = Setup::new();
    std::fs::create_dir_all(s.project().join(".claude")).unwrap();
    std::fs::write(
        s.project_settings(),
        r#"{"theme":"dark","hooks":{"Stop":[{"hooks":[{"type":"command","command":"/usr/bin/audit"}]}]}}"#,
    )
    .unwrap();
    s.init(&with(&["--stop-warn"]), "");
    let v = Setup::read(&s.project_settings());
    assert_eq!(v["theme"], "dark");
    assert!(cmds(&v, "Stop").iter().any(|c| c == "/usr/bin/audit"));
    s.init(&with(&["--uninstall"]), "");
    let v = Setup::read(&s.project_settings());
    assert_eq!(cmds(&v, "Stop"), vec!["/usr/bin/audit".to_string()]);
}

#[test]
fn codex_refuses_it() {
    let s = Setup::new();
    let o = s.init(&["--codex", "--yes", "--stop-warn"], "");
    assert!(!o.status.success());
}

#[test]
fn consent_shows_and_switches_the_stop_warning() {
    let s = Setup::new();
    s.init(&ARGS, "");
    let shown = out(&s.klatch("consent", &["--scope", "project"], ""));
    assert!(shown.contains("stop-warn: off"), "{shown}");
    s.klatch(
        "consent",
        &["stop-warn", "on", "--scope", "project", "--yes"],
        "",
    );
    let v = Setup::read(&s.project_settings());
    assert_eq!(has_flag(&v, "PostToolUse", "--stop-warn"), vec![true]);
    let shown = out(&s.klatch("consent", &["--scope", "project"], ""));
    assert!(
        shown.contains("stop-warn: on") && shown.contains("machine-status: off"),
        "{shown}"
    );
    s.klatch(
        "consent",
        &["stop-warn", "off", "--scope", "project", "--yes"],
        "",
    );
    let shown = out(&s.klatch("consent", &["--scope", "project"], ""));
    assert!(shown.contains("stop-warn: off"), "{shown}");
}

fn stop(h: &Home, flag: bool, extra: &str) -> String {
    let proj = h.0.join("proj");
    std::fs::create_dir_all(proj.join(".git")).unwrap();
    let payload = format!(
        r#"{{"session_id":"s-alice","hook_event_name":"Stop","cwd":"{}"{extra}}}"#,
        proj.display()
    );
    let mut args = vec!["hook", "--event", "Stop"];
    if flag {
        args.push("--stop-warn");
    }
    let o = hook_with_payload(h, Some("alice"), &payload, &args);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    out(&o)
}

#[test]
fn a_stop_hook_that_carries_the_flag_prints_one_system_message_and_one_without_it_prints_nothing() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    let running =
        r#","background_tasks":[{"type":"shell","command":"SECRET-CMD"}],"session_crons":[]"#;
    let text = stop(&h, true, running);
    assert!(
        text.contains("systemMessage") && text.contains("shell 1"),
        "{text}"
    );
    assert!(!text.contains("SECRET"), "{text}");
    assert_eq!(text.lines().count(), 1, "{text}");
    assert_eq!(stop(&h, false, running), "");
    assert_eq!(stop(&h, true, ""), "");
    assert_eq!(
        stop(&h, true, r#","background_tasks":"x","session_crons":{}"#),
        ""
    );
}

#[test]
fn a_stop_inside_a_subagent_prints_nothing_even_with_the_flag() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    let extra = r#","agent_id":"sub-1","background_tasks":[{"type":"shell"}],"session_crons":[]"#;
    assert_eq!(stop(&h, true, extra), "");
}

#[test]
fn a_huge_list_prints_the_same_capped_count_as_a_huge_number() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    let list = vec!["{}"; 12000].join(",");
    let as_list = format!(r#","background_tasks":[{list}],"session_crons":[{list}]"#);
    let as_number = r#","background_tasks":50000,"session_crons":50000"#;
    for extra in [as_list.as_str(), as_number] {
        let text = stop(&h, true, extra);
        assert!(
            text.contains("9999 background task(s) (other 9999) and 9999 cron(s)"),
            "{text}"
        );
    }
    let exact = r#","background_tasks":9999,"session_crons":9998"#;
    assert!(
        stop(&h, true, exact).contains("9999 background task(s) (other 9999) and 9998 cron(s)")
    );
}
