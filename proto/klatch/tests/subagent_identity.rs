//! #503: a subagent that posts must give itself a name. A coding-agent process below a session that has no
//! registry record of its own would, under --discover, take the session's member name and so share its cursor and
//! author with every sibling. Discovery refuses to guess a shared identity for such a child; a top level session
//! is unchanged, and a child that sets KLATCH_NAME is its own member. Real processes: programs named `claude` (a
//! symlink to the shell) with a registry record for the session only. Home and registry are temporary.
#![cfg(unix)]
mod common;

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

/// A klatch command line as a process in a session runs it: no typed name unless `name` gives one.
fn klatch(h: &Home, fake: &Path, name: Option<&str>, args: &str) -> String {
    let name = name.map_or("-u KLATCH_NAME".to_string(), |n| format!("KLATCH_NAME={n}"));
    format!(
        "env -u KLATCH_IDENTITY_FILE {name} HOME='{}' KLATCH_HOME='{}' KLATCH_KEY_DIR='{}' '{}' {args}",
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

#[test]
fn a_child_under_a_session_is_refused_without_a_name_and_the_message_says_what_to_set() {
    let h = Home::new();
    h.ok("obs", &["join", "lobby"]);
    let fake = fake_home(&h);
    let body = klatch(&h, &fake, None, "--discover post lobby hi");
    let run = child(&h, "a", &body);
    let (_, text) = finish(session(&h, &fake, &format!("{run}; true")));
    assert!(text.contains("KLATCH_NAME"), "{text}");
    assert!(text.contains("subagent"), "{text}");
    assert!(
        !h.ok("obs", &["read", "lobby"]).contains("hi"),
        "the refused child posted"
    );
}

#[test]
fn a_read_in_a_child_is_refused_too_so_it_never_eats_the_parents_unread_mail() {
    let h = Home::new();
    h.ok("obs", &["join", "lobby"]);
    let fake = fake_home(&h);
    let body = klatch(&h, &fake, None, "--discover read lobby");
    let run = child(&h, "r", &body);
    let (_, text) = finish(session(&h, &fake, &format!("{run}; true")));
    assert!(text.contains("KLATCH_NAME"), "{text}");
}

#[test]
fn a_child_with_a_name_is_its_own_member_and_siblings_do_not_hide_each_others_posts() {
    let h = Home::new();
    let fake = fake_home(&h);
    let mut script = String::new();
    for (tag, name) in [("one", "sub-one"), ("two", "sub-two")] {
        let body = format!(
            "{}\n{}\n",
            klatch(&h, &fake, Some(name), "--discover join lobby"),
            klatch(
                &h,
                &fake,
                Some(name),
                &format!("--discover post lobby from-{tag}")
            ),
        );
        script += &format!("{}; ", child(&h, tag, &body));
    }
    let reader = child(
        &h,
        "read",
        &klatch(&h, &fake, Some("sub-two"), "--discover read lobby"),
    );
    script += &format!("{reader}; true");
    let (_, text) = finish(session(&h, &fake, &script));
    assert!(text.contains("from-one"), "sibling two never saw one: {text}");
    let who = h.ok("obs", &["who", "lobby", "--all"]);
    assert!(who.contains("sub-one") && who.contains("sub-two"), "{who}");
    assert!(!who.contains("parent"), "the parent member appeared: {who}");
}

#[test]
fn a_top_level_session_is_unchanged_and_keeps_its_discovered_name() {
    let h = Home::new();
    let fake = fake_home(&h);
    let script = klatch(&h, &fake, None, "--discover join lobby");
    let (ok, text) = finish(session(&h, &fake, &script));
    assert!(ok, "{text}");
    assert!(text.contains("parent joined lobby"), "{text}");
}
