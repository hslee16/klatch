//! `klatch doctor` (#66): a read-only health check. It says what works, what does not and why, and how to fix it. It
//! creates nothing (no store, no key), and it never prints a key or a recovery code.
mod common;

use common::*;
use std::process::Command;

/// Run `klatch doctor` in the setup's project directory, with a key directory of its own so no real keystore is touched.
fn doctor(s: &Setup) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_klatch"))
        .current_dir(s.project())
        .env_clear()
        .env("HOME", s.home())
        .env("KLATCH_KEY_DIR", s.home().join("keys"))
        .arg("doctor")
        .output()
        .unwrap()
}

fn text(o: &std::process::Output) -> String {
    out(o) + &String::from_utf8_lossy(&o.stderr)
}

fn line<'a>(t: &'a str, area: &str) -> &'a str {
    t.lines()
        .find(|l| l.contains(&format!("{area}:")))
        .unwrap_or_else(|| panic!("no '{area}' line in:\n{t}"))
}

#[test]
fn a_fresh_machine_is_healthy_and_says_what_is_missing_without_failing() {
    let s = Setup::new();
    let o = doctor(&s);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(
        line(&t, "binary").starts_with("ok") && line(&t, "binary").contains("protocol"),
        "{t}"
    );
    assert!(line(&t, "store").contains("no store yet"), "{t}");
    assert!(
        line(&t, "hooks").starts_with("warn") && line(&t, "hooks").contains("not installed"),
        "{t}"
    );
    assert!(
        line(&t, "session").contains("not inside a coding-agent session"),
        "{t}"
    );
    assert!(t.contains("0 failures"), "{t}");
}

#[test]
fn doctor_creates_nothing() {
    let s = Setup::new();
    doctor(&s);
    doctor(&s);
    assert!(
        !s.home().join(".klatch").exists(),
        "doctor created a store directory"
    );
    assert!(
        !s.home().join("keys").exists(),
        "doctor created a key directory"
    );
    assert!(
        !s.home().join(".claude").exists(),
        "doctor wrote into the harness's directory"
    );
    assert!(
        !s.project().join(".claude").exists(),
        "doctor wrote settings"
    );
}

#[test]
fn installed_hooks_are_reported_as_ok_and_the_optional_ones_as_on_or_off() {
    let s = Setup::new();
    assert!(
        s.init(&["--scope", "project", "--yes", "--wake"], "")
            .status
            .success()
    );
    let t = text(&doctor(&s));
    let hooks = line(&t, "hooks (project)");
    assert!(
        hooks.starts_with("ok") && hooks.contains("SessionStart"),
        "{t}"
    );
    assert!(line(&t, "wake hook").contains("on"), "{t}");
    assert!(line(&t, "enforce hook").contains("off"), "{t}");
}

#[test]
fn a_hook_that_points_at_a_binary_that_is_gone_is_a_failure_with_the_fix() {
    let s = Setup::new();
    std::fs::create_dir_all(s.project().join(".claude")).unwrap();
    std::fs::write(
        s.project_settings(),
        r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"'/nonexistent/place/klatch' --discover hook --event SessionStart"}]}]}}"#,
    )
    .unwrap();
    let o = doctor(&s);
    let t = text(&o);
    assert!(!o.status.success(), "{t}");
    let l = line(&t, "hooks (project)");
    assert!(
        l.starts_with("FAIL")
            && l.contains("/nonexistent/place/klatch")
            && l.contains("does not exist"),
        "{t}"
    );
    assert!(t.contains("klatch init"), "no fix named: {t}");
}

#[test]
fn a_hook_that_points_at_another_klatch_is_a_warning() {
    let s = Setup::new();
    let other = s.home().join("other-klatch");
    std::fs::copy(env!("CARGO_BIN_EXE_klatch"), &other).unwrap();
    std::fs::create_dir_all(s.project().join(".claude")).unwrap();
    std::fs::write(
        s.project_settings(),
        format!(
            r#"{{"hooks":{{"SessionStart":[{{"hooks":[{{"type":"command","command":"'{}' --discover hook --event SessionStart"}}]}}]}}}}"#,
            other.display()
        ),
    )
    .unwrap();
    let o = doctor(&s);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(
        line(&t, "hooks (project)").starts_with("warn")
            && line(&t, "hooks (project)").contains("not this binary"),
        "{t}"
    );
}

#[test]
fn a_plaintext_store_is_a_warning_that_says_how_it_is_converted() {
    let s = Setup::new();
    std::fs::create_dir_all(s.home().join(".klatch")).unwrap();
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/store/prototype-v2.db"
        ),
        s.home().join(".klatch/rooms.db"),
    )
    .unwrap();
    let before = std::fs::read(s.home().join(".klatch/rooms.db")).unwrap();
    let o = doctor(&s);
    let t = text(&o);
    assert!(
        line(&t, "store").starts_with("warn") && line(&t, "store").contains("not encrypted"),
        "{t}"
    );
    assert_eq!(
        std::fs::read(s.home().join(".klatch/rooms.db")).unwrap(),
        before,
        "doctor changed the store"
    );
    assert!(!s.home().join("keys").exists(), "doctor made a key");
}

#[test]
fn an_encrypted_store_with_its_key_is_ok_and_without_it_is_a_failure_that_names_recovery() {
    let s = Setup::new();
    // make a real encrypted store and its key by using klatch once
    let home = s.home();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_klatch"))
            .env_clear()
            .env("HOME", &home)
            .env("KLATCH_KEY_DIR", home.join("keys"))
            .env("KLATCH_NAME", "alice")
            .args(args)
            .output()
            .unwrap()
    };
    assert!(run(&["join", "lobby"]).status.success());
    let t = text(&doctor(&s));
    assert!(
        line(&t, "store").starts_with("ok") && line(&t, "store").contains("encrypted"),
        "{t}"
    );
    // no key, no recovery: move the key files away
    std::fs::rename(home.join("keys"), home.join("keys-gone")).unwrap();
    let o = doctor(&s);
    let t = text(&o);
    assert!(!o.status.success(), "{t}");
    assert!(
        line(&t, "store").starts_with("FAIL") && t.contains("klatch admin recover"),
        "{t}"
    );
}

#[test]
fn doctor_never_prints_a_key() {
    let s = Setup::new();
    let home = s.home();
    Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env_clear()
        .env("HOME", &home)
        .env("KLATCH_KEY_DIR", home.join("keys"))
        .env("KLATCH_NAME", "alice")
        .args(["join", "lobby"])
        .output()
        .unwrap();
    // the key file's content, in any form it could be printed
    let mut secrets = Vec::new();
    for entry in std::fs::read_dir(home.join("keys")).unwrap().flatten() {
        let bytes = std::fs::read(entry.path()).unwrap();
        secrets.push(String::from_utf8_lossy(&bytes).trim().to_string());
        secrets.push(bytes.iter().map(|b| format!("{b:02x}")).collect::<String>());
    }
    let t = text(&doctor(&s));
    for secret in secrets.iter().filter(|s| s.len() >= 16) {
        assert!(!t.contains(secret.as_str()), "doctor printed key material");
    }
    assert!(!t.contains("KLATCH-"), "doctor printed a recovery code");
}
