//! Golden text of the discovery refusals (#1480): what a subagent (a coding-agent process below a session that has no
//! registry record of its own) is told when it asks to be a member without a name (#503), what a process with no
//! session above it is told under `--discover`, and the refusal to set the first admin passphrase from inside a
//! coding-agent session (#1233). Real processes: programs named `claude` (a symlink to the shell) with a registry
//! record for the session only; home and registry are throwaway. Regenerate on purpose only: `KLATCH_WRITE_GOLDEN=1`.
#![cfg(unix)]
mod common;

use common::admin::*;
use common::golden::*;
use common::*;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

fn domain() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else {
        std::env::consts::OS
    }
}

fn lstart(pid: u32) -> String {
    let out = Command::new("ps")
        .env("TZ", "UTC")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn fake_home(h: &Home) -> PathBuf {
    let f = h.0.join("fake-home");
    std::fs::create_dir_all(&f).unwrap();
    f
}

/// A `claude` session with a registry record, running `script` once the record is written.
fn session(h: &Home, fake: &Path, script: &str) -> Child {
    let bin = h.0.join("claude");
    let _ = std::fs::remove_file(&bin);
    std::os::unix::fs::symlink("/bin/sh", &bin).unwrap();
    let ready = h.0.join("ready");
    let _ = std::fs::remove_file(&ready);
    let script = format!(
        "while [ ! -f '{}' ]; do sleep 0.02; done; {script}",
        ready.display()
    );
    let child = Command::new(&bin)
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let dir = fake.join(".claude").join("sessions");
    std::fs::create_dir_all(&dir).unwrap();
    let pid = child.id();
    for _ in 0..100 {
        if !lstart(pid).is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let record = format!(
        r#"{{"pid":{pid},"sessionId":"sess-parent","cwd":"/w","name":"parent","procStart":"{}","pidDomain":"{}"}}"#,
        lstart(pid),
        domain()
    );
    std::fs::write(dir.join(format!("{pid}.json")), record).unwrap();
    std::fs::write(ready, "").unwrap();
    child
}

/// A klatch command line as a process in a session runs it: no typed name.
fn klatch(h: &Home, fake: &Path, args: &str) -> String {
    format!(
        "env -u KLATCH_IDENTITY_FILE -u KLATCH_NAME HOME='{}' KLATCH_HOME='{}' KLATCH_KEY_DIR='{}' '{}' {args}",
        fake.display(),
        h.0.display(),
        h.0.join("keys").display(),
        env!("CARGO_BIN_EXE_klatch")
    )
}

/// A command that runs `body` in a second `claude` process (a subagent: no registry record), started from the session.
fn child(h: &Home, tag: &str, body: &str) -> String {
    let file = h.0.join(format!("child-{tag}.sh"));
    std::fs::write(&file, body).unwrap();
    format!("'{}' '{}'", h.0.join("claude").display(), file.display())
}

fn finish(c: Child) -> (bool, String) {
    let o = c.wait_with_output().unwrap();
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr),
    )
}

fn sheet(h: &Home) -> Sheet {
    Sheet::new(Scrub::new().path(&h.0, "<HOME>").binary())
}

#[test]
fn a_subagent_without_a_name_is_told_what_to_set_for_every_command_it_may_try() {
    let h = Home::new();
    h.ok("obs", &["join", "lobby"]);
    let fake = fake_home(&h);
    let mut s = sheet(&h);
    for (tag, args) in [
        ("post", "--discover post lobby hi"),
        ("read", "--discover read lobby"),
        ("join", "--discover join lobby"),
        ("who", "--discover who lobby"),
    ] {
        let run = child(&h, tag, &klatch(&h, &fake, args));
        let (_, text) = finish(session(&h, &fake, &format!("{run}; true")));
        s.note(&format!("a subagent runs: klatch {args}"), &text);
    }
    assert!(
        !h.ok("obs", &["read", "lobby"]).contains("hi"),
        "the refused child posted"
    );
    s.check("discovery_subagent_refusals", &[PASS, WRONG, NEW]);
}

#[test]
fn a_process_with_no_session_above_it_is_told_so_under_discover() {
    let h = Home::new();
    let fake = fake_home(&h);
    let mut s = sheet(&h);
    for args in [
        ["--discover", "join", "lobby"],
        ["--discover", "post", "lobby"],
    ] {
        let mut c = h.launcher_command(&args);
        c.env("HOME", &fake);
        let o = c.detached().output().unwrap();
        let text =
            String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr);
        s.block(
            &format!("no session: klatch {}", args.join(" ")),
            o.status.success(),
            &text,
        );
    }
    s.check("discovery_no_session", &[PASS, WRONG, NEW]);
}

#[test]
fn the_first_passphrase_is_not_set_from_inside_a_coding_agent_session() {
    let h = Home::new();
    let fake = fake_home(&h);
    let mut s = sheet(&h);
    let body = klatch(&h, &fake, "admin init-passphrase");
    let (_, text) = finish(session(&h, &fake, &format!("{body}; true")));
    s.note(
        "a session runs: klatch admin init-passphrase (no terminal)",
        &text,
    );
    // with a passphrase set, a session's admin command has no terminal and is refused for that
    let h2 = Home::new();
    h2.set_passphrase();
    let fake2 = fake_home(&h2);
    s.scrub_path(&h2.0, "<HOME>");
    let body = klatch(&h2, &fake2, "admin lease-cap 100");
    let (_, text) = finish(session(&h2, &fake2, &format!("{body}; true")));
    s.note(
        "a session runs: klatch admin lease-cap 100 (no terminal)",
        &text,
    );
    s.check("discovery_admin_in_session", &[PASS, WRONG, NEW]);
}
