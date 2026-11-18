//! A child agent that runs a klatch binary other than the installed one, and fails, is told which binary is the
//! installed one (#513). `klatch convention --room` writes the brief for a child in a room other than the lobby.
mod common;

use common::*;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Put a klatch hook that runs `program` in the user settings of `s`.
fn install(s: &Setup, program: &Path) {
    let settings = s.user_settings();
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    let cmd = format!("{} --discover hook --event SessionStart", program.display());
    let json = format!(
        r#"{{"hooks":{{"SessionStart":[{{"hooks":[{{"type":"command","command":"{cmd}"}}]}}]}}}}"#
    );
    std::fs::write(settings, json).unwrap();
}

/// A file that stands for the installed binary: it only has to exist.
fn fake_installed(s: &Setup) -> PathBuf {
    let dir = s.home().join("installed-klatch");
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("klatch");
    std::fs::write(&bin, "").unwrap();
    bin
}

/// Run a command that fails (the store is not a database) as `s`'s home and project.
fn failing(s: &Setup) -> Output {
    let store = s.home().join("store");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::write(store.join("rooms.db"), b"not a database, just bytes").unwrap();
    Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env_remove("KLATCH_IDENTITY_FILE")
        .env_clear()
        .current_dir(s.project())
        .env("HOME", s.home())
        .env("KLATCH_HOME", &store)
        .env("KLATCH_KEY_DIR", s.home().join("keys"))
        .env("KLATCH_NAME", "child")
        .args(["read", "lobby"])
        .output()
        .unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn a_failure_names_the_installed_binary_when_the_one_that_ran_is_another() {
    let s = Setup::new();
    let installed = fake_installed(&s);
    install(&s, &installed);
    let o = failing(&s);
    assert!(!o.status.success());
    let e = stderr(&o);
    assert!(e.contains("installed klatch"), "{e}");
    assert!(e.contains(&installed.display().to_string()), "{e}");
    assert!(e.contains(env!("CARGO_BIN_EXE_klatch")), "{e}");
    assert!(e.contains("convention"), "it points at the brief: {e}");
}

#[test]
fn no_note_when_the_installed_binary_is_the_one_that_ran() {
    let s = Setup::new();
    install(&s, Path::new(env!("CARGO_BIN_EXE_klatch")));
    let e = stderr(&failing(&s));
    assert!(!e.contains("installed klatch"), "{e}");
}

#[test]
fn no_note_when_nothing_is_installed_or_the_installed_program_is_gone() {
    let s = Setup::new();
    assert!(!stderr(&failing(&s)).contains("installed klatch"));
    install(&s, &s.home().join("gone").join("klatch"));
    assert!(!stderr(&failing(&s)).contains("installed klatch"));
}

#[test]
fn the_convention_for_a_room_other_than_the_lobby_names_that_room_everywhere() {
    let o = Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env_remove("KLATCH_IDENTITY_FILE")
        .env_clear()
        .args(["convention", "--name", "fixer-1", "--room", "ask-7"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", stderr(&o));
    let t = String::from_utf8(o.stdout).unwrap();
    assert!(!t.contains("lobby"), "{t}");
    let bin = env!("CARGO_BIN_EXE_klatch");
    assert!(
        t.contains(&format!("KLATCH_NAME=fixer-1 {bin} --discover join ask-7")),
        "{t}"
    );
}

#[test]
fn a_room_name_that_could_carry_a_shell_command_is_refused() {
    let o = Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env_remove("KLATCH_IDENTITY_FILE")
        .env_clear()
        .args(["convention", "--room", "x; rm -rf ~"])
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(stderr(&o).contains("room"));
}
