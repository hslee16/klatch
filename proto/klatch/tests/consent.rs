//! `klatch consent` (#256): show the consents `klatch init` granted and switch each on or off later, exactly and
//! reversibly, touching only what klatch wrote. Every test runs the real binary in a throwaway project and home.
mod common;

use common::*;

const FOREIGN: &str = r#"{"zeta":1,"hooks":{"PostToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"/usr/bin/audit"}]}],"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"/usr/bin/guard"}]}],"Stop":[{"hooks":[{"type":"command","command":"/usr/bin/mine"}]}]}}"#;

fn installed() -> Setup {
    let s = Setup::new();
    let o = s.init(&["--scope", "project", "--yes"], "");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    s
}

fn text(s: &Setup) -> String {
    std::fs::read_to_string(s.project_settings()).unwrap()
}

fn set(s: &Setup, name: &str, value: &str) -> std::process::Output {
    s.klatch("consent", &[name, value, "--scope", "project", "--yes"], "")
}

fn show(s: &Setup) -> String {
    let o = s.klatch("consent", &[], "");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    out(&o)
}

#[test]
fn show_says_nothing_is_installed_before_init_and_writes_nothing() {
    let s = Setup::new();
    let shown = show(&s);
    assert!(
        shown.contains("project") && shown.contains("user") && shown.contains("not installed"),
        "{shown}"
    );
    assert!(!s.project_settings().exists() && !s.user_settings().exists());
}

#[test]
fn show_reports_each_consent_for_project_and_user_scope() {
    let s = installed();
    let b = s.init(
        &["--scope", "user", "--yes", "--board", "lobby", "--no-lobby"],
        "",
    );
    assert!(b.status.success());
    let shown = show(&s);
    let (project, user) = shown.split_once("\nuser").expect("a user section");
    for line in [
        "board: off",
        "join: on",
        "session-end: on",
        "enforce: off",
        "wake: off",
    ] {
        assert!(project.contains(line), "project lacks {line}: {shown}");
    }
    for line in [
        "board: lobby",
        "join: off",
        "session-end: on",
        "enforce: off",
        "wake: off",
    ] {
        assert!(user.contains(line), "user lacks {line}: {shown}");
    }
}

#[test]
fn every_consent_switches_off_and_on_again_byte_for_byte() {
    let s = installed();
    let start = text(&s);
    for (name, on, off) in [
        ("join", "on", "off"),
        ("session-end", "on", "off"),
        ("enforce", "off", "on"),
        ("wake", "off", "on"),
        ("board", "off", "lobby"),
    ] {
        let flipped = set(&s, name, off);
        assert!(
            flipped.status.success(),
            "{name} {off}: {}",
            String::from_utf8_lossy(&flipped.stderr)
        );
        assert_ne!(text(&s), start, "{name} {off} changed nothing");
        let back = set(&s, name, on);
        assert!(
            back.status.success(),
            "{name} {on}: {}",
            String::from_utf8_lossy(&back.stderr)
        );
        assert_eq!(text(&s), start, "{name} did not come back exactly");
    }
}

#[test]
fn a_change_is_reported_by_show() {
    let s = installed();
    set(&s, "join", "off");
    set(&s, "board", "lobby");
    set(&s, "wake", "on");
    let shown = show(&s);
    assert!(
        shown.contains("join: off") && shown.contains("board: lobby") && shown.contains("wake: on"),
        "{shown}"
    );
}

#[test]
fn an_entry_klatch_did_not_write_is_never_touched() {
    let s = Setup::new();
    std::fs::create_dir_all(s.project().join(".claude")).unwrap();
    std::fs::write(s.project_settings(), FOREIGN).unwrap();
    s.init(&["--scope", "project", "--yes"], "");
    for (name, value) in [
        ("join", "off"),
        ("board", "lobby"),
        ("enforce", "on"),
        ("wake", "on"),
        ("session-end", "off"),
        ("enforce", "off"),
        ("wake", "off"),
        ("board", "off"),
        ("join", "on"),
        ("session-end", "on"),
    ] {
        assert!(set(&s, name, value).status.success(), "{name} {value}");
        let v = Setup::read(&s.project_settings());
        assert_eq!(v["zeta"], 1);
        assert!(hook_cmds(&v, "PostToolUse").contains(&"/usr/bin/audit".to_string()));
        assert!(hook_cmds(&v, "PreToolUse").contains(&"/usr/bin/guard".to_string()));
        assert!(hook_cmds(&v, "Stop").contains(&"/usr/bin/mine".to_string()));
    }
}

#[test]
fn nothing_is_added_where_init_did_not_install_klatch() {
    let s = Setup::new();
    let o = set(&s, "join", "off");
    assert!(!o.status.success());
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("klatch init"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(!s.project_settings().exists(), "created a settings file");
    std::fs::create_dir_all(s.project().join(".claude")).unwrap();
    std::fs::write(s.project_settings(), FOREIGN).unwrap();
    assert!(!set(&s, "enforce", "on").status.success());
    assert_eq!(text(&s), FOREIGN, "a file with no klatch hook was changed");
}

#[test]
fn a_change_without_a_scope_or_with_a_bad_name_or_value_is_refused() {
    let s = installed();
    let start = text(&s);
    for args in [
        vec!["join", "off", "--yes"],
        vec!["nonsense", "on", "--scope", "project", "--yes"],
        vec!["join", "maybe", "--scope", "project", "--yes"],
        vec!["board", "a b", "--scope", "project", "--yes"],
        vec!["join", "--scope", "project", "--yes"],
    ] {
        let o = s.klatch("consent", &args, "");
        assert!(!o.status.success(), "{args:?} was accepted");
        assert_eq!(text(&s), start, "{args:?} wrote");
    }
}

#[test]
fn without_yes_it_shows_the_diff_and_a_no_writes_nothing() {
    let s = installed();
    let start = text(&s);
    let o = s.klatch("consent", &["join", "off", "--scope", "project"], "n\n");
    assert!(o.status.success());
    assert!(
        out(&o).contains("+ ") && out(&o).contains("--no-join"),
        "{}",
        out(&o)
    );
    assert_eq!(text(&s), start, "wrote without a yes");
    let y = s.klatch("consent", &["join", "off", "--scope", "project"], "y\n");
    assert!(y.status.success());
    assert!(text(&s).contains("--no-join"));
}

#[test]
fn a_change_says_it_takes_effect_at_the_next_session() {
    let s = installed();
    let o = set(&s, "join", "off");
    assert!(out(&o).contains("next session"), "{}", out(&o));
}

#[test]
fn the_user_scope_is_changed_only_when_asked_for() {
    let s = installed();
    s.init(&["--scope", "user", "--yes"], "");
    let user = std::fs::read_to_string(s.user_settings()).unwrap();
    set(&s, "join", "off");
    assert_eq!(std::fs::read_to_string(s.user_settings()).unwrap(), user);
    let o = s.klatch("consent", &["wake", "on", "--scope", "both", "--yes"], "");
    assert!(o.status.success());
    assert!(
        std::fs::read_to_string(s.user_settings())
            .unwrap()
            .contains("asyncRewake")
    );
    assert!(text(&s).contains("asyncRewake"));
}

#[test]
fn a_settings_file_that_is_not_json_is_refused_and_left_alone() {
    let s = installed();
    std::fs::write(s.project_settings(), "{ not json").unwrap();
    let o = set(&s, "join", "off");
    assert!(!o.status.success());
    assert_eq!(text(&s), "{ not json");
}
