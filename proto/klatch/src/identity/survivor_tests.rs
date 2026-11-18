//! Tests for the session identity paths that a mutation run found unpinned (#724, from #407): the session id of a hook's
//! input, the member names of sessions that collide, and the parent of a session. The parent lookup reads the harness
//! registry under `$HOME`, so it runs in a child process of the test binary with a temporary `HOME`.

use super::*;
use klatch::harness::{ProcessTable, PsTable};
use std::process::{Command, Stdio};

fn store(tag: &str) -> SqliteStore {
    let dir = std::env::temp_dir().join(format!(
        "klatch-survivor-names-{}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    SqliteStore::open(&dir.join("klatch.db"), &klatch::store::insecure_test_key()).unwrap()
}

/// Run `f` on its own thread and fail, instead of hanging, when it does not finish.
fn bounded<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(std::time::Duration::from_secs(30))
        .expect("the member name search did not finish")
}

fn member(s: &SqliteStore, sid: &str, name: &str) -> String {
    ensure_session_member(s, sid, Some(name), 1).unwrap()
}

#[test]
fn the_session_id_of_a_hook_input_is_its_text_session_id_and_nothing_else() {
    assert_eq!(
        session_id_of(&serde_json::json!({"session_id": "abc-123"})).as_deref(),
        Some("abc-123")
    );
    assert_eq!(session_id_of(&serde_json::json!({})), None);
    assert_eq!(session_id_of(&serde_json::json!({"session_id": 7})), None);
    assert_eq!(
        session_id_of(&serde_json::json!({"session_id": null})),
        None
    );
    assert_eq!(session_id_of(&serde_json::json!("abc")), None);
}

#[test]
fn sessions_that_collide_on_a_name_each_get_a_longer_suffix_until_the_name_is_free() {
    let names = bounded(|| {
        let s = store("chain");
        [
            member(&s, "sid-aaaa1111", "proj-20"),
            member(&s, "sid-bbbb2222", "proj-20"),
            member(&s, "sid-bbbb3333", "proj-20"),
            member(&s, "sid-bbbb4444", "proj-20"),
        ]
    });
    assert_eq!(
        names,
        ["proj-20", "proj-20-sidb", "proj-20-sidbb", "proj-20-sidbbb"]
    );
}

#[test]
fn a_forty_character_name_stays_within_forty_characters_through_every_suffix_length() {
    let long = "n".repeat(40);
    let names = bounded(move || {
        let s = store("long-chain");
        [
            member(&s, "sid-aaaa1111", &long),
            member(&s, "sid-bbbb2222", &long),
            member(&s, "sid-bbbb3333", &long),
            member(&s, "sid-bbbb4444", &long),
        ]
    });
    assert_eq!(names[0], "n".repeat(40));
    assert_eq!(names[1], format!("{}-sidb", "n".repeat(35)));
    assert_eq!(names[2], format!("{}-sidbb", "n".repeat(34)));
    assert_eq!(names[3], format!("{}-sidbbb", "n".repeat(33)));
    assert!(names.iter().all(|n| n.len() == 40), "{names:?}");
}

#[test]
fn a_session_id_longer_than_the_suffix_grows_it_by_one_character_when_four_collide() {
    let (first, second) = bounded(|| {
        let s = store("five");
        member(&s, "x", "w");
        member(&s, "abcd1", "w");
        (member(&s, "abcd5", "w"), member(&s, "abcd5", "w"))
    });
    assert_eq!(first, "w-abcd5");
    assert_eq!(second, first, "the name is kept for the session's life");
}

#[test]
fn a_session_id_with_too_few_characters_to_lengthen_still_gets_a_name() {
    let (a, b) = bounded(|| {
        let s = store("short");
        member(&s, "first", "x");
        (member(&s, "ab", "x"), member(&s, "a-b", "x"))
    });
    assert_eq!(a, "x-ab");
    // the second session gets the same name as the first (a known gap, #767); this
    // pins only that the search ends and the name is valid
    assert!(b.starts_with("x-") && b.len() <= 40, "{b}");
}

#[test]
fn the_member_of_a_found_session_is_named_remembered_and_returned() {
    let s = store("member-of");
    let found = Found {
        session_id: "sid-member-of".into(),
        name: Some("alice-7".into()),
        cwd: "/w".into(),
        pid: 5,
    };
    assert_eq!(member_of(&s, &found).as_deref(), Some("alice-7"));
    assert_eq!(
        s.member_of_session("sid-member-of").unwrap().as_deref(),
        Some("alice-7")
    );
    let unnamed = Found {
        session_id: "0123456789abcdef".into(),
        name: None,
        ..found
    };
    assert_eq!(member_of(&s, &unnamed).as_deref(), Some("session-01234567"));
}

#[test]
fn a_caller_with_no_session_has_no_parent_to_look_for() {
    let s = store("no-session");
    let caller = caller_of("kid", None);
    assert_eq!(pending_parent(&s, &caller), None);
}

fn caller_of(member: &str, session: Option<Found>) -> Caller {
    Caller {
        principal: Principal::unattributed(member),
        session,
        session_dir: None,
        stdin_session: None,
        stdin: None,
    }
}

fn found_at(pid: u32) -> Found {
    Found {
        session_id: "sid-kid".into(),
        name: Some("kid".into()),
        cwd: "/w".into(),
        pid,
    }
}

#[test]
fn a_caller_whose_parent_was_already_noted_is_not_looked_up_again() {
    let s = store("noted");
    klatch::store::MemberStore::note_parent(&s, "kid", "boss", 1).unwrap();
    let caller = caller_of("kid", Some(found_at(std::process::id())));
    assert_eq!(pending_parent(&s, &caller), None);
}

#[test]
fn a_session_with_no_session_above_it_records_that_it_has_no_parent() {
    let d = std::env::temp_dir().join(format!(
        "klatch-survivor-parent-{}-none",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    in_child("child_no_parent", &d);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_session_started_from_another_session_names_that_session_as_its_parent() {
    let d = std::env::temp_dir().join(format!(
        "klatch-survivor-parent-{}-boss",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    in_child("child_parent_found", &d);
    let _ = std::fs::remove_dir_all(&d);
}

fn in_child(name: &str, dir: &std::path::Path) {
    let out = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("identity::survivor_tests::{name}"),
            "--test-threads=1",
        ])
        .env_clear()
        .env("KLATCH_SURVIVOR_DIR", dir)
        .env("HOME", dir.join("home"))
        .env("TMPDIR", std::env::temp_dir())
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{name} failed in the child:\n{text}");
    assert!(
        text.contains("test result: ok. 1 passed"),
        "{name} did not run in the child:\n{text}"
    );
}

fn child_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("KLATCH_SURVIVOR_DIR").map(Into::into)
}

#[test]
fn child_no_parent() {
    let Some(_) = child_dir() else { return };
    let s = store("child-none");
    let caller = caller_of("kid", Some(found_at(std::process::id())));
    assert_eq!(pending_parent(&s, &caller), Some(None));
    assert!(discover_parent(&found_at(std::process::id())).is_none());
}

/// A process the harness registry can honestly call a running Claude Code: a copy of the shell named `claude`, which
/// starts a `sleep` as its child. Returns it and the pid of the child.
fn fake_harness(dir: &std::path::Path) -> (std::process::Child, u32) {
    let exe = dir.join("claude");
    std::os::unix::fs::symlink("/bin/sh", &exe).unwrap();
    let mut shell = Command::new(&exe)
        .args(["-c", "sleep 5 & echo $!; wait"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(shell.stdout.take().unwrap()),
        &mut line,
    )
    .unwrap();
    (shell, line.trim().parse().unwrap())
}

#[test]
fn child_parent_found() {
    let Some(d) = child_dir() else { return };
    let (mut shell, kid_pid) = fake_harness(&d);
    let shell_pid = shell.id();
    let verdict = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        check_parent(&d, shell_pid, kid_pid)
    }));
    // the shell is killed; its `sleep` child ends by itself within seconds
    let _ = shell.kill();
    let _ = shell.wait();
    if let Err(e) = verdict {
        std::panic::resume_unwind(e);
    }
}

fn check_parent(d: &std::path::Path, shell_pid: u32, kid_pid: u32) {
    let info = PsTable.info(shell_pid).expect("the fake harness runs");
    assert!(
        info.command.ends_with("claude"),
        "this host shows the fake harness as {:?}, not claude",
        info.command
    );
    let domain = if cfg!(target_os = "macos") {
        "darwin"
    } else {
        std::env::consts::OS
    };
    let sessions = d.join("home").join(".claude").join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let record = serde_json::json!({
        "pid": shell_pid, "sessionId": "sid-boss", "cwd": "/w", "name": "boss-name",
        "procStart": info.start_utc, "pidDomain": domain,
    });
    std::fs::write(
        sessions.join(format!("{shell_pid}.json")),
        record.to_string(),
    )
    .unwrap();
    let found = discover_parent(&found_at(kid_pid)).expect("the session above was not found");
    assert_eq!(found.session_id, "sid-boss");
    let s = store("child-boss");
    let caller = caller_of("kid", Some(found_at(kid_pid)));
    assert_eq!(
        pending_parent(&s, &caller),
        Some(Some("boss-name".to_string()))
    );
    assert_eq!(
        s.member_of_session("sid-boss").unwrap().as_deref(),
        Some("boss-name"),
        "the parent was not remembered as a member"
    );
}
