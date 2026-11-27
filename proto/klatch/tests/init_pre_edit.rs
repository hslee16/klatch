//! `klatch init --pre-edit-warn` and `--pre-edit-claims` (#1402, #1458): the consents live in one PreToolUse hook entry of
//! their own, separate from the blocking `--enforce` entry. Fixture settings files in a throwaway project and home only.
mod common;

use common::*;

const ARGS: [&str; 3] = ["--scope", "project", "--yes"];

fn with(extra: &[&'static str]) -> Vec<&'static str> {
    let mut v = ARGS.to_vec();
    v.extend_from_slice(extra);
    v
}

fn pre(s: &Setup) -> Vec<String> {
    let v = Setup::read(&s.project_settings());
    v["hooks"]["PreToolUse"]
        .as_array()
        .map_or_else(Vec::new, |_| hook_cmds(&v, "PreToolUse"))
}

fn matchers(s: &Setup) -> Vec<String> {
    let v = Setup::read(&s.project_settings());
    v["hooks"]["PreToolUse"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| m["matcher"].as_str().unwrap_or("").to_string())
        .collect()
}

fn has(cmd: &str, flag: &str) -> bool {
    cmd.split(' ').any(|w| w == flag)
}

#[test]
fn it_is_off_unless_asked_for_and_the_plain_install_has_no_pre_tool_use_hook() {
    let s = Setup::new();
    s.init(&ARGS, "");
    assert!(pre(&s).is_empty());
}

#[test]
fn the_warning_flag_adds_its_own_pre_tool_use_entry_for_the_edit_tools_only() {
    let s = Setup::new();
    let o = s.init(&with(&["--pre-edit-warn"]), "");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let cmds = pre(&s);
    assert_eq!(cmds.len(), 1, "{cmds:?}");
    assert!(
        cmds[0].ends_with("--discover hook --event PreToolUse --pre-edit-warn"),
        "{}",
        cmds[0]
    );
    assert_eq!(matchers(&s), vec!["Edit|Write|MultiEdit|NotebookEdit"]);
    // nothing else changed: no passive claims, no Stop hook
    let v = Setup::read(&s.project_settings());
    assert!(v["hooks"].get("Stop").is_none());
    assert!(!has(&hook_cmds(&v, "PostToolUse")[0], "--passive-claims"));
}

#[test]
fn a_plain_rerun_keeps_it_and_no_pre_edit_warn_takes_it_out_with_the_claims() {
    let s = Setup::new();
    s.init(&with(&["--passive-claims", "--pre-edit-claims"]), "");
    let before = std::fs::read(s.project_settings()).unwrap();
    let o = s.init(&ARGS, "");
    assert!(out(&o).contains("already up to date"), "{}", out(&o));
    assert_eq!(std::fs::read(s.project_settings()).unwrap(), before);
    s.init(&with(&["--no-pre-edit-warn"]), "");
    assert!(pre(&s).is_empty());
}

#[test]
fn the_claims_flag_needs_passive_claims_and_implies_the_warning() {
    let s = Setup::new();
    let o = s.init(&with(&["--pre-edit-claims"]), "");
    assert!(!o.status.success(), "{}", out(&o));
    let err = String::from_utf8_lossy(&o.stderr).into_owned();
    assert!(err.contains("--passive-claims"), "{err}");
    assert!(!s.project_settings().exists(), "nothing was written");
    let o = s.init(&with(&["--passive-claims", "--pre-edit-claims"]), "");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let cmds = pre(&s);
    assert_eq!(cmds.len(), 1);
    assert!(has(&cmds[0], "--pre-edit-warn") && has(&cmds[0], "--pre-edit-claims"));
    // the claims come off alone and leave the warning
    s.init(&with(&["--no-pre-edit-claims"]), "");
    let cmds = pre(&s);
    assert!(has(&cmds[0], "--pre-edit-warn") && !has(&cmds[0], "--pre-edit-claims"));
}

#[test]
fn turning_passive_claims_off_drops_the_first_edit_claim_and_keeps_the_warning() {
    let s = Setup::new();
    s.init(&with(&["--passive-claims", "--pre-edit-claims"]), "");
    s.init(&with(&["--no-passive-claims"]), "");
    let cmds = pre(&s);
    assert_eq!(cmds.len(), 1);
    assert!(has(&cmds[0], "--pre-edit-warn") && !has(&cmds[0], "--pre-edit-claims"));
}

#[test]
fn it_never_touches_the_enforce_hook_and_enforce_never_touches_it() {
    let s = Setup::new();
    s.init(&with(&["--enforce", "--pre-edit-warn"]), "");
    let cmds = pre(&s);
    assert_eq!(cmds.len(), 2, "{cmds:?}");
    assert!(cmds.iter().any(|c| c.ends_with("--event PreToolUse")));
    assert!(cmds.iter().any(|c| has(c, "--pre-edit-warn")));
    s.init(&with(&["--no-enforce"]), "");
    let cmds = pre(&s);
    assert_eq!(cmds.len(), 1, "{cmds:?}");
    assert!(has(&cmds[0], "--pre-edit-warn"));
    s.init(&with(&["--enforce", "--no-pre-edit-warn"]), "");
    let cmds = pre(&s);
    assert_eq!(cmds.len(), 1, "{cmds:?}");
    assert!(!has(&cmds[0], "--pre-edit-warn"));
}

#[test]
fn other_pre_tool_use_hooks_stay_and_uninstall_takes_klatchs_out() {
    let s = Setup::new();
    std::fs::create_dir_all(s.project().join(".claude")).unwrap();
    std::fs::write(
        s.project_settings(),
        r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"/usr/bin/guard"}]}]}}"#,
    )
    .unwrap();
    s.init(&with(&["--pre-edit-warn"]), "");
    assert_eq!(pre(&s).len(), 2);
    s.init(&with(&["--no-pre-edit-warn"]), "");
    assert_eq!(pre(&s), vec!["/usr/bin/guard".to_string()]);
    s.init(&with(&["--pre-edit-warn"]), "");
    s.init(&["--scope", "project", "--yes", "--uninstall"], "");
    assert_eq!(pre(&s), vec!["/usr/bin/guard".to_string()]);
}

#[test]
fn the_consent_command_shows_and_switches_both_and_the_legend_says_what_they_do() {
    let s = Setup::new();
    s.init(&with(&["--passive-claims"]), "");
    let o = s.klatch("consent", &["--scope", "project"], "");
    let text = out(&o);
    assert!(
        text.contains("pre-edit-warn: off") && text.contains("pre-edit-claims: off"),
        "{text}"
    );
    let o = s.klatch(
        "consent",
        &["pre-edit-warn", "on", "--scope", "project", "--yes"],
        "",
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(has(&pre(&s)[0], "--pre-edit-warn"));
    let o = s.klatch(
        "consent",
        &["pre-edit-claims", "on", "--scope", "project", "--yes"],
        "",
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(has(&pre(&s)[0], "--pre-edit-claims"));
    let text = out(&s.klatch("consent", &["--scope", "project"], ""));
    assert!(
        text.contains("pre-edit-warn: on") && text.contains("pre-edit-claims: on"),
        "{text}"
    );
    let o = s.klatch(
        "consent",
        &["passive-claims", "off", "--scope", "project", "--yes"],
        "",
    );
    assert!(o.status.success());
    assert!(!has(&pre(&s)[0], "--pre-edit-claims"));
    let o = s.klatch(
        "consent",
        &["pre-edit-claims", "on", "--scope", "project", "--yes"],
        "",
    );
    assert!(!o.status.success(), "claims need passive claims");
}

#[test]
fn doctor_reports_the_pre_edit_warning_hook_and_codex_refuses_the_flags() {
    let s = Setup::new();
    s.init(&with(&["--pre-edit-warn"]), "");
    let o = s.klatch("doctor", &[], "");
    let text = out(&o);
    assert!(text.contains("pre-edit-warn hook: on"), "{text}");
    let plain = Setup::new();
    plain.init(&ARGS, "");
    let text = out(&plain.klatch("doctor", &[], ""));
    assert!(
        text.contains("pre-edit-warn hook: off (klatch init --pre-edit-warn turns it on)"),
        "{text}"
    );
    for flag in ["--pre-edit-warn", "--pre-edit-claims"] {
        let o = plain.init(&["--codex", flag, "--yes"], "");
        assert!(!o.status.success(), "{flag}");
    }
}
