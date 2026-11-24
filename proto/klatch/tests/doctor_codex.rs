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
    c.env_remove("KLATCH_IDENTITY_FILE");
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
        &format!(
            "[hooks.state.\"{key}\"]\ntrusted_hash = \"sha256:1055e4969f2467390aa1f1af05f9c325b70f1fbd5f27c654eb5fdbff8ae4626d\"\n"
        ),
    );
    assert!(line(&doctor(&s, None, Some(&codex)), "codex hooks:").starts_with("ok"));
    write(
        &codex.join("config.toml"),
        &format!(
            "[hooks.state.\"{}:session_start:0:1\"]\ntrusted_hash = \"sha256:1055e4969f2467390aa1f1af05f9c325b70f1fbd5f27c654eb5fdbff8ae4626d\"\n",
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

// ---- #590: key indexes, inline and project hooks, a changed hook, async hooks. The hashes below were produced by
// Codex itself (`codex app-server`, `hooks/list`, RESEARCH-NOTES 49.7), except the no-matcher one, computed by the same rule.

const KLATCH_SESSION_START: &str = "/opt/klatch/klatch --discover hook --event SessionStart";
/// The hash of the SessionStart klatch hook with no matcher by the rule that reproduced five real Codex hashes.
const HASH_SESSION_START: &str =
    "sha256:1055e4969f2467390aa1f1af05f9c325b70f1fbd5f27c654eb5fdbff8ae4626d";
/// ... with the matcher `startup`.
const HASH_SESSION_START_STARTUP: &str =
    "sha256:b7e2c8af3709751cf204a8346551891a6a5707fb48c9b60c42a085ca932338c2";
/// Codex's `currentHash` for the inline `[[hooks.Stop]]` klatch hook in config.toml.
const HASH_STOP: &str = "sha256:6743c9ef4e6ebb218e996df434f804d5eab719b02bc0eba6ec6e2ffa9c701df9";

fn trust(file: &Path, (key_tail, hash): (&str, &str)) -> String {
    format!(
        "[hooks.state.\"{}:{key_tail}\"]\ntrusted_hash = \"{hash}\"\n",
        file.display()
    )
}

#[test]
fn a_file_with_several_groups_is_judged_by_the_group_and_handler_index() {
    let s = Setup::new();
    let codex = s.home().join("codexhome");
    let hooks = codex.join("hooks.json");
    let other = r#"{"type":"command","command":"/bin/echo other"}"#;
    write(
        &hooks,
        &format!(
            r#"{{"hooks":{{"SessionStart":[{{"hooks":[{other}]}},{{"matcher":"startup","hooks":[{other},{{"type":"command","command":"{KLATCH_SESSION_START}"}}]}}]}}}}"#
        ),
    );
    // Codex keys the klatch hook session_start:1:1 (group 1, handler 1)
    write(
        &codex.join("config.toml"),
        &trust(&hooks, ("session_start:1:1", HASH_SESSION_START_STARTUP)),
    );
    let t = doctor(&s, None, Some(&codex));
    assert!(line(&t, "codex hooks:").starts_with("ok"), "{t}");
    // a record for the first slot does not cover it
    write(
        &codex.join("config.toml"),
        &trust(&hooks, ("session_start:0:0", HASH_SESSION_START_STARTUP)),
    );
    let t = doctor(&s, None, Some(&codex));
    assert!(line(&t, "codex hooks:").starts_with("warn"), "{t}");
}

#[test]
fn a_changed_hook_with_an_old_hash_is_not_trusted() {
    let s = Setup::new();
    let codex = s.home().join("codexhome");
    let hooks = codex.join("hooks.json");
    write(&hooks, &hooks_json(KLATCH_HOOK));
    write(
        &codex.join("config.toml"),
        &trust(&hooks, ("session_start:0:0", HASH_SESSION_START)),
    );
    let t = doctor(&s, None, Some(&codex));
    assert!(line(&t, "codex hooks:").starts_with("ok"), "{t}");
    // the same record, but the command was edited after it was trusted
    write(
        &hooks,
        &hooks_json("/opt/klatch/klatch --discover hook --event SessionStart --extra"),
    );
    let t = doctor(&s, None, Some(&codex));
    let l = line(&t, "codex hooks:");
    assert!(l.starts_with("warn"), "{t}");
    assert!(l.contains("changed since") && l.contains("trusted"), "{t}");
    assert!(t.contains("/hooks"), "{t}");
}

#[test]
fn a_hook_written_inline_in_config_toml_is_read_too() {
    let s = Setup::new();
    let codex = s.home().join("codexhome");
    let config = codex.join("config.toml");
    let inline = "[[hooks.Stop]]\n[[hooks.Stop.hooks]]\ntype = \"command\"\ncommand = \"/opt/klatch/klatch --discover hook --event Stop\"\n";
    write(&config, inline);
    let t = doctor(&s, None, Some(&codex));
    let l = line(&t, "codex hooks:");
    assert!(l.starts_with("warn") && l.contains("config.toml"), "{t}");
    write(
        &config,
        &format!("{inline}\n{}", trust(&config, ("stop:0:0", HASH_STOP))),
    );
    let t = doctor(&s, None, Some(&codex));
    assert!(line(&t, "codex hooks:").starts_with("ok"), "{t}");
}

#[test]
fn an_async_klatch_hook_is_reported_because_codex_skips_it() {
    let s = Setup::new();
    let codex = s.home().join("codexhome");
    write(
        &codex.join("hooks.json"),
        &format!(
            r#"{{"hooks":{{"SessionStart":[{{"hooks":[{{"type":"command","command":"{KLATCH_SESSION_START}","async":true}}]}}]}}}}"#
        ),
    );
    let t = doctor(&s, None, Some(&codex));
    let l = line(&t, "codex hooks:");
    assert!(l.starts_with("warn"), "{t}");
    assert!(l.contains("async") && l.contains("skips"), "{t}");
}

/// `config.toml` text marking `project` trusted the way Codex records it (RESEARCH-NOTES 49.9).
fn project_trusted(project: &Path, level: &str) -> String {
    format!(
        "[projects.\"{}\"]\ntrust_level = \"{level}\"\n",
        project.display()
    )
}

/// A project `.codex/hooks.json` with one klatch hook, a Codex home that marks the project `level` and holds
/// `record` (a trusted hash for the hook, or none); returns the doctor output.
fn project_doctor(level: Option<&str>, record: Option<&str>, command: &str) -> String {
    let s = Setup::new();
    let codex = s.home().join("codexhome");
    write(&s.project().join(".codex/hooks.json"), &hooks_json(command));
    // Codex is started in the real directory, so its key and its project record use the real spelling
    let project = s.project().canonicalize().unwrap();
    let hooks = project.join(".codex/hooks.json");
    let mut config = level.map_or(String::new(), |l| project_trusted(&project, l));
    if let Some(hash) = record {
        config += &trust(&hooks, ("session_start:0:0", hash));
    }
    write(&codex.join("config.toml"), &config);
    doctor(&s, None, Some(&codex))
}

#[test]
fn a_trusted_project_hook_with_a_matching_record_is_ok() {
    let t = project_doctor(Some("trusted"), Some(HASH_SESSION_START), KLATCH_HOOK);
    let l = line(&t, "codex hooks (project):");
    assert!(
        l.starts_with("ok") && l.contains(".codex/hooks.json"),
        "{t}"
    );
}

#[test]
fn a_trusted_project_hook_with_no_record_is_reported_with_the_fix() {
    let t = project_doctor(Some("trusted"), None, KLATCH_HOOK);
    let l = line(&t, "codex hooks (project):");
    assert!(l.starts_with("warn"), "{t}");
    assert!(
        l.contains("not trusted by Codex") && l.contains("SessionStart"),
        "{t}"
    );
    assert!(t.contains("/hooks"), "{t}");
}

#[test]
fn a_trusted_project_hook_changed_since_it_was_trusted_is_reported() {
    let changed = "/opt/klatch/klatch --discover hook --event SessionStart --extra";
    let t = project_doctor(Some("trusted"), Some(HASH_SESSION_START), changed);
    let l = line(&t, "codex hooks (project):");
    assert!(l.starts_with("warn") && l.contains("changed since"), "{t}");
    assert!(t.contains("/hooks"), "{t}");
}

#[test]
fn a_project_not_recorded_as_trusted_is_said_to_be_unchecked() {
    // Codex does not load the project layer, so it lists no hook and keeps no record that means anything
    for level in [None, Some("untrusted")] {
        let t = project_doctor(level, Some(HASH_SESSION_START), KLATCH_HOOK);
        let l = line(&t, "codex hooks (project):");
        assert!(l.starts_with("warn"), "{t}");
        assert!(
            l.contains("cannot be checked") && l.contains("not recorded as trusted"),
            "{t}"
        );
        assert!(t.contains("/hooks"), "{t}");
    }
}
