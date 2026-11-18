//! `klatch init --codex` (#782): writes klatch's synchronous hooks into Codex's hooks.json with the same consent and
//! diff as the Claude Code installer, never the wake hook, never a trust record. Every test runs the real binary with a
//! throwaway home and CODEX_HOME; the real ~/.codex is never read or written.
mod common;

use common::*;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

struct Codex {
    setup: Setup,
}

impl Codex {
    fn new() -> Self {
        Codex {
            setup: Setup::new(),
        }
    }
    fn dir(&self) -> PathBuf {
        self.setup.home().join("codex-here")
    }
    fn file(&self) -> PathBuf {
        self.dir().join("hooks.json")
    }
    /// Run `klatch init --codex ...` with CODEX_HOME set (or not) in an empty environment.
    fn init(&self, args: &[&str], stdin: &str, codex_home: Option<&PathBuf>) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_klatch"));
        cmd.env_remove("KLATCH_IDENTITY_FILE")
            .env_clear()
            .current_dir(self.setup.project())
            .env("HOME", self.setup.home())
            .arg("init")
            .arg("--codex")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = codex_home {
            cmd.env("CODEX_HOME", dir);
        }
        let mut child = cmd.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
    fn yes(&self, args: &[&str]) -> Output {
        let mut all = vec!["--yes"];
        all.extend_from_slice(args);
        self.init(&all, "", Some(&self.dir()))
    }
    fn text(&self) -> String {
        std::fs::read_to_string(self.file()).unwrap()
    }
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

const FOREIGN: &str = r#"{"zeta":1,"hooks":{"PostToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"/usr/bin/audit"}]}],"Stop":[{"hooks":[{"type":"command","command":"/usr/bin/mine"}]}]}}"#;

#[test]
fn it_writes_the_three_sync_hooks_with_the_codex_flag_and_nothing_async() {
    let c = Codex::new();
    let o = c.yes(&[]);
    assert!(o.status.success(), "{}", err(&o));
    let v = Setup::read(&c.file());
    let bin = env!("CARGO_BIN_EXE_klatch");
    for e in ["SessionStart", "UserPromptSubmit", "PostToolUse"] {
        assert_eq!(
            hook_cmds(&v, e),
            vec![format!("{bin} --codex hook --event {e}")],
            "{e}"
        );
    }
    let events: Vec<&String> = v["hooks"].as_object().unwrap().keys().collect();
    assert_eq!(events.len(), 3, "an event beyond the three: {events:?}");
    let raw = c.text();
    assert!(
        !raw.contains("async") && !raw.contains("--discover"),
        "{raw}"
    );
}

#[test]
fn it_writes_only_the_codex_file_and_never_the_claude_settings() {
    let c = Codex::new();
    c.yes(&[]);
    assert!(!c.setup.project_settings().exists() && !c.setup.user_settings().exists());
    assert!(!c.setup.project().join(".codex").exists());
    let names: Vec<_> = std::fs::read_dir(c.dir())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("hooks.json")]);
}

#[test]
fn without_codex_home_it_uses_dot_codex_under_the_home() {
    let c = Codex::new();
    let o = c.init(&["--yes"], "", None);
    assert!(o.status.success(), "{}", err(&o));
    assert!(c.setup.home().join(".codex/hooks.json").exists());
}

#[test]
fn it_is_safe_to_run_again_and_keeps_every_other_hook_and_setting() {
    let c = Codex::new();
    std::fs::create_dir_all(c.dir()).unwrap();
    std::fs::write(c.file(), FOREIGN).unwrap();
    c.yes(&[]);
    let once = c.text();
    let again = c.yes(&[]);
    assert!(
        out(&again).contains("already up to date"),
        "{}",
        out(&again)
    );
    assert_eq!(c.text(), once);
    let v = Setup::read(&c.file());
    assert_eq!(v["zeta"], 1);
    assert!(hook_cmds(&v, "PostToolUse").contains(&"/usr/bin/audit".to_string()));
    assert!(hook_cmds(&v, "Stop").contains(&"/usr/bin/mine".to_string()));
    assert_eq!(hook_cmds(&v, "PostToolUse").len(), 2);
}

#[test]
fn the_board_and_the_lobby_choice_go_into_session_start_only() {
    let c = Codex::new();
    c.yes(&["--board", "lobby", "--no-lobby"]);
    let v = Setup::read(&c.file());
    let start = &hook_cmds(&v, "SessionStart")[0];
    assert!(
        start.ends_with("--event SessionStart --board lobby --no-join"),
        "{start}"
    );
    assert!(!hook_cmds(&v, "PostToolUse")[0].contains("--board"));
}

#[test]
fn it_ends_with_the_trust_step_and_says_the_wake_hook_is_left_out() {
    let c = Codex::new();
    let o = c.yes(&[]);
    let said = out(&o);
    assert!(said.contains("/hooks"), "{said}");
    assert!(said.contains("wake"), "{said}");
    assert!(!said.contains("--dangerously-bypass-hook-trust --") || said.contains("never"));
    assert!(
        !c.dir().join("config.toml").exists(),
        "wrote a trust record or a config"
    );
}

#[test]
fn it_asks_and_shows_the_diff_and_a_no_writes_nothing() {
    let c = Codex::new();
    let o = c.init(&[], "n\ny\n", Some(&c.dir()));
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        out(&o).contains("+ ") && out(&o).contains("--codex hook --event"),
        "{}",
        out(&o)
    );
    assert!(!c.file().exists(), "wrote without a yes");
    let y = c.init(&[], "y\ny\ny\n", Some(&c.dir()));
    assert!(y.status.success());
    assert!(c.file().exists());
}

#[test]
fn uninstall_removes_only_klatchs_codex_hooks() {
    let c = Codex::new();
    std::fs::create_dir_all(c.dir()).unwrap();
    std::fs::write(c.file(), FOREIGN).unwrap();
    c.yes(&[]);
    let o = c.yes(&["--uninstall"]);
    assert!(o.status.success(), "{}", err(&o));
    let v = Setup::read(&c.file());
    assert_eq!(v["zeta"], 1);
    assert_eq!(hook_cmds(&v, "PostToolUse"), vec!["/usr/bin/audit"]);
    assert_eq!(hook_cmds(&v, "Stop"), vec!["/usr/bin/mine"]);
    assert!(v["hooks"].get("SessionStart").is_none());
}

#[test]
fn a_file_that_is_not_json_is_refused_and_left_alone() {
    let c = Codex::new();
    std::fs::create_dir_all(c.dir()).unwrap();
    std::fs::write(c.file(), "{ not json").unwrap();
    let o = c.yes(&[]);
    assert!(!o.status.success());
    assert_eq!(c.text(), "{ not json");
}

#[test]
fn the_claude_only_options_are_refused_with_codex() {
    let c = Codex::new();
    for flag in [
        vec!["--scope", "project"],
        vec!["--wake"],
        vec!["--enforce"],
        vec!["--no-session-end"],
    ] {
        let mut args = vec!["--yes"];
        args.extend(flag.clone());
        let o = c.init(&args, "", Some(&c.dir()));
        assert!(!o.status.success(), "{flag:?} was accepted with --codex");
    }
    assert!(!c.file().exists());
}
