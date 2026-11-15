//! `klatch doctor` and `klatch init` on a machine with Codex (#544, #545, RESEARCH-NOTES 49.3). Codex skips a hook
//! that was never trusted without saying so in `codex exec`, so a klatch hook there would look healthy and deliver
//! nothing; doctor reads the Codex config (never writes it) and says so. The wake path `codex queue` is found by
//! running `codex queue --help`, never by version. Every test uses a temp HOME or CODEX_HOME and a fake `codex`.
mod common;

use common::*;
use std::path::{Path, PathBuf};
use std::process::Command;

const KLATCH_HOOK: &str = "/opt/klatch/klatch --discover hook --event SessionStart";

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn hooks_json(command: &str) -> String {
    format!(
        r#"{{"hooks":{{"SessionStart":[{{"hooks":[{{"type":"command","command":"{command}"}}]}}]}}}}"#
    )
}

/// A directory holding a `codex` that answers `queue --help` with `queue_help` and everything else with the top
/// level usage, like the 0.147.0 binary that has no queue command.
fn fake_codex(s: &Setup, queue_help: &str) -> PathBuf {
    let dir = s.home().join("bin");
    let script = format!(
        "#!/bin/sh\ncase \"$*\" in\n  \"queue --help\") printf '%s\\n' '{queue_help}' ;;\n  \"--version\") printf 'codex-cli 0.147.0\\n' ;;\n  *) printf 'Usage: codex [OPTIONS] [PROMPT]\\n' ;;\nesac\n"
    );
    let bin = dir.join("codex");
    write(&bin, &script);
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

fn doctor(s: &Setup, path: Option<&Path>, codex_home: Option<&Path>) -> String {
    let mut c = Command::new(env!("CARGO_BIN_EXE_klatch"));
    c.current_dir(s.project())
        .env_clear()
        .env("HOME", s.home())
        .env("KLATCH_KEY_DIR", s.home().join("keys"))
        .env(
            "PATH",
            path.map_or(s.home().join("nowhere"), Path::to_path_buf),
        )
        .arg("doctor");
    if let Some(h) = codex_home {
        c.env("CODEX_HOME", h);
    }
    let o = c.output().unwrap();
    out(&o) + &String::from_utf8_lossy(&o.stderr)
}

fn line<'a>(t: &'a str, prefix: &str) -> &'a str {
    t.lines()
        .find(|l| l.contains(prefix))
        .unwrap_or_else(|| panic!("no '{prefix}' line in:\n{t}"))
}

#[test]
fn an_untrusted_klatch_hook_in_codex_is_reported_as_skipped_silently_with_the_fix() {
    let s = Setup::new();
    let codex = s.home().join("codexhome");
    write(&codex.join("hooks.json"), &hooks_json(KLATCH_HOOK));
    let t = doctor(&s, None, Some(&codex));
    let l = line(&t, "codex hooks:");
    assert!(l.starts_with("warn"), "{t}");
    assert!(
        l.contains("not trusted") && l.contains("skipped silently"),
        "{t}"
    );
    assert!(
        t.contains("/hooks"),
        "the fix must say where to trust it: {t}"
    );
    assert!(
        t.contains("never"),
        "must say klatch never bypasses or writes trust: {t}"
    );
    // read only: the config was not created and the hooks file is as it was
    assert!(
        !codex.join("config.toml").exists(),
        "doctor wrote the Codex config"
    );
    assert_eq!(
        std::fs::read_to_string(codex.join("hooks.json")).unwrap(),
        hooks_json(KLATCH_HOOK)
    );
}

#[test]
fn a_trusted_klatch_hook_is_ok_and_a_trust_record_for_another_slot_does_not_count() {
    let s = Setup::new();
    let codex = s.home().join("codexhome");
    write(&codex.join("hooks.json"), &hooks_json(KLATCH_HOOK));
    let key = format!("{}:session_start:0:0", codex.join("hooks.json").display());
    write(
        &codex.join("config.toml"),
        &format!("[hooks.state.\"{key}\"]\ntrusted_hash = \"sha256:abc\"\n"),
    );
    assert!(line(&doctor(&s, None, Some(&codex)), "codex hooks:").starts_with("ok"));
    write(
        &codex.join("config.toml"),
        &format!(
            "[hooks.state.\"{}:session_start:0:1\"]\ntrusted_hash = \"sha256:abc\"\n",
            codex.join("hooks.json").display()
        ),
    );
    assert!(line(&doctor(&s, None, Some(&codex)), "codex hooks:").starts_with("warn"));
}

#[test]
fn a_record_with_no_hash_is_not_trust() {
    let s = Setup::new();
    let codex = s.home().join("codexhome");
    write(&codex.join("hooks.json"), &hooks_json(KLATCH_HOOK));
    let key = format!("{}:session_start:0:0", codex.join("hooks.json").display());
    write(
        &codex.join("config.toml"),
        &format!("[hooks.state.\"{key}\"]\n"),
    );
    assert!(line(&doctor(&s, None, Some(&codex)), "codex hooks:").starts_with("warn"));
}

#[test]
fn codex_hooks_that_are_not_klatchs_are_not_klatchs_business() {
    let s = Setup::new();
    let codex = s.home().join("codexhome");
    write(
        &codex.join("hooks.json"),
        &hooks_json("/usr/bin/atuin hook codex"),
    );
    let t = doctor(&s, None, Some(&codex));
    assert!(!t.contains("codex hooks:"), "{t}");
}

#[test]
fn the_default_codex_home_is_dot_codex_under_home() {
    let s = Setup::new();
    write(
        &s.home().join(".codex/hooks.json"),
        &hooks_json(KLATCH_HOOK),
    );
    assert!(line(&doctor(&s, None, None), "codex hooks:").starts_with("warn"));
}

#[test]
fn the_queue_command_is_found_by_probing_and_not_by_version() {
    let s = Setup::new();
    let none = fake_codex(&s, "Usage: codex [OPTIONS] [PROMPT]");
    let t = doctor(&s, Some(&none), None);
    let l = line(&t, "codex queue:");
    assert!(
        l.contains("no queue subcommand") && l.contains("codex queue --help"),
        "{t}"
    );
    assert!(
        l.contains("0.147.0"),
        "the version is shown for information: {t}"
    );

    let s2 = Setup::new();
    let has = fake_codex(
        &s2,
        "Usage: codex queue [OPTIONS] --thread <ID> --message <TEXT>",
    );
    let t = doctor(&s2, Some(&has), None);
    let l = line(&t, "codex queue:");
    assert!(
        l.starts_with("ok") && l.contains("has a queue subcommand"),
        "{t}"
    );
    assert!(l.contains("exit 0 is not proof of delivery"), "{t}");
}

#[test]
fn no_codex_on_the_path_is_said_plainly_and_is_not_a_warning() {
    let s = Setup::new();
    let t = doctor(&s, None, None);
    let l = line(&t, "codex:");
    assert!(l.starts_with("ok") && l.contains("not installed"), "{t}");
}

#[test]
fn init_reports_an_untrusted_klatch_hook_in_codex_and_writes_nothing_there() {
    let s = Setup::new();
    let codex = s.home().join(".codex");
    write(&codex.join("hooks.json"), &hooks_json(KLATCH_HOOK));
    let o = s.init(&["--scope", "project", "--yes"], "");
    let t = out(&o);
    assert!(
        o.status.success(),
        "{t}{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(
        t.contains("Codex") && t.contains("not trusted") && t.contains("/hooks"),
        "{t}"
    );
    assert!(
        !codex.join("config.toml").exists(),
        "init wrote the Codex config"
    );
}
