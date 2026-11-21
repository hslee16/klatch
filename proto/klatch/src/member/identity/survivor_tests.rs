//! Tests for the session identity paths that a mutation run found unpinned (#724, from #407): the session id of a hook's
//! input, the member names of sessions that collide, and the parent of a session. The parent lookup reads the harness
//! registry under `$HOME`, so it runs in a child process of the test binary with a temporary `HOME`.

use super::*;
use crate::harness::{ProcessTable, PsTable};
use std::process::{Command, Stdio};

fn store(tag: &str) -> crate::store::test_dir::In<SqliteStore> {
    crate::store::test_dir::In::new(&format!("survivor-names-{tag}"), |dir| {
        SqliteStore::open(&dir.join("klatch.db"), &crate::store::insecure_test_key()).unwrap()
    })
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

/// A store that only keeps which session holds which member name, and counts how often it is asked who holds one.
#[derive(Default)]
struct Holders {
    held: std::cell::RefCell<std::collections::HashMap<String, String>>,
    lookups: std::cell::Cell<usize>,
}

/// The required methods of the areas this store does not keep: nothing in the name search may reach them.
impl crate::store::areas::MessageStore for Holders {
    fn join(&self, _: &str, _: &str, _: i64) -> crate::domain::Result<()> {
        unimplemented!()
    }
    fn post_as(
        &self,
        _: &crate::store::NewMessage,
        _: i64,
    ) -> crate::domain::Result<crate::domain::Posted> {
        unimplemented!()
    }
    fn read(
        &self,
        _: crate::store::Seat,
        _: bool,
        _: i64,
    ) -> crate::domain::Result<Vec<crate::domain::Message>> {
        unimplemented!()
    }
    fn tail(
        &self,
        _: &str,
        _: i64,
        _: usize,
    ) -> crate::domain::Result<Vec<crate::domain::Message>> {
        unimplemented!()
    }
    fn head(&self, _: &str) -> crate::domain::Result<i64> {
        unimplemented!()
    }
    fn advance_cursor(
        &self,
        _: crate::store::Seat,
        _: crate::store::Seq,
        _: i64,
    ) -> crate::domain::Result<()> {
        unimplemented!()
    }
}
impl crate::store::areas::AskStore for Holders {}
impl crate::store::areas::ClaimStore for Holders {
    fn claim(&self, _: &crate::store::ClaimRequest, _: i64) -> crate::domain::Result<()> {
        unimplemented!()
    }
    fn release(&self, _: &str, _: &str, _: &str) -> crate::domain::Result<()> {
        unimplemented!()
    }
    fn claims(&self, _: &str, _: i64) -> crate::domain::Result<Vec<crate::domain::Claim>> {
        unimplemented!()
    }
}
impl crate::store::areas::ReviewStore for Holders {}
impl crate::store::areas::CredentialStore for Holders {}
impl crate::store::areas::PolicyStore for Holders {}
impl crate::store::areas::RoomStore for Holders {
    fn features(&self) -> crate::store::Features {
        crate::store::Features::none()
    }
    fn set_label_keys(&self, _: crate::keys::LabelKeys) {
        unimplemented!()
    }
}
impl crate::store::areas::MemberStore for Holders {
    fn members(&self, _: &str) -> crate::domain::Result<Vec<crate::domain::MemberInfo>> {
        unimplemented!()
    }
    fn rooms_of(&self, _: &str) -> crate::domain::Result<Vec<String>> {
        unimplemented!()
    }

    fn session_of_member(&self, member: &str) -> crate::domain::Result<Option<String>> {
        self.lookups.set(self.lookups.get() + 1);
        Ok(self.held.borrow().get(member).cloned())
    }

    fn note_session(&self, session_id: &str, member: &str, _now: i64) -> crate::domain::Result<()> {
        self.held
            .borrow_mut()
            .insert(member.into(), session_id.into());
        Ok(())
    }
}

/// #801: when every suffix length up to the whole id is taken, the search must stop lengthening at the id (it has no
/// more characters to give) and count up. The name that comes out is the same one if the loop runs one round too many,
/// so what pins the bound is the number of times the store is asked: one lookup per name tried, no repeat of the
/// full-id name.
#[test]
fn the_suffix_search_stops_at_the_whole_id_and_asks_once_per_name_it_tries() {
    let s = Holders::default();
    for taken in ["proj", "proj-abcd", "proj-abcde"] {
        s.note_session("someone-else", taken, 1).unwrap();
    }
    let got = ensure_session_member(&s, "abcde", Some("proj"), 1).unwrap();
    assert_eq!(got, "proj-abcde-2");
    // proj, proj-abcd, proj-abcde (twice: the end of the growing loop and the first round of the counting loop),
    // proj-abcde-2
    assert_eq!(s.lookups.get(), 5, "the full-id name was asked about again");
    assert_eq!(s.held.borrow().get(&got).map(String::as_str), Some("abcde"));
}

#[test]
fn a_session_id_with_too_few_characters_to_lengthen_still_gets_a_name() {
    let (a, b, b_again) = bounded(|| {
        let s = store("short");
        member(&s, "first", "x");
        let a = member(&s, "ab", "x");
        let b = member(&s, "a-b", "x");
        (a, b, member(&s, "a-b", "x"))
    });
    assert_eq!(a, "x-ab", "an existing name never changes");
    assert_ne!(b, a, "two sessions never share a member name (#767)");
    assert_ne!(b, "x");
    assert!(crate::domain::valid_name(&b), "{b}");
    assert_eq!(b_again, b, "the name is kept for the session's life");
}

#[test]
fn sessions_whose_ids_run_out_of_characters_never_share_a_name_even_when_many_collide() {
    let long_id = "z".repeat(60);
    let names = bounded(move || {
        let s = store("exhausted");
        let mut names = vec![member(&s, "first", "x")];
        for sid in [
            "ab",
            "a-b",
            "a_b",
            "A-B",
            "---",
            "!!!",
            "",
            &long_id,
            &format!("{long_id}1"),
        ] {
            names.push(member(&s, sid, "x"));
        }
        // every name is still the session's name when asked again
        for (i, sid) in ["ab", "a-b", "a_b", "A-B", "---", "!!!", ""]
            .iter()
            .enumerate()
        {
            assert_eq!(&member(&s, sid, "x"), &names[i + 1], "{sid}");
        }
        names
    });
    let unique: std::collections::HashSet<_> = names.iter().collect();
    assert_eq!(unique.len(), names.len(), "{names:?}");
    assert!(
        names.iter().all(|n| crate::domain::valid_name(n)),
        "{names:?}"
    );
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
        claude_code: false,
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
    crate::store::MemberStore::note_parent(&*s, "kid", "boss", 1).unwrap();
    let caller = caller_of("kid", Some(found_at(std::process::id())));
    assert_eq!(pending_parent(&s, &caller), None);
}

#[test]
fn a_session_with_no_session_above_it_records_that_it_has_no_parent() {
    let d = crate::store::test_dir::TestDir::new("survivor-parent-none");
    in_child("child_no_parent", &d);
}

#[test]
fn a_session_started_from_another_session_names_that_session_as_its_parent() {
    let d = crate::store::test_dir::TestDir::new("survivor-parent-boss");
    in_child("child_parent_found", &d);
}

fn in_child(name: &str, dir: &std::path::Path) {
    let out = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("member::identity::survivor_tests::{name}"),
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
