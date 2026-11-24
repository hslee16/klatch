use super::*;
use crate::harness::liveness::fake::{FakeTable, UnreadableTable};
use crate::harness::process::Lookup;
use crate::harness::record::{FsRegistry, parse_record};
use crate::harness::{Capability, Harness, Level};
use std::os::unix::fs::PermissionsExt;

const T: &str = "Fri Nov 6 03:04:05 2026";
const D: &str = "darwin";

// hook (30) -> shell (20) -> codex (10) -> terminal (5) -> launchd (1)
fn tree() -> FakeTable {
    FakeTable::default()
        .with((30, 20), "Sat Nov 7 10:00:00 2026", "klatch")
        .with((20, 10), "Sat Nov 7 09:59:59 2026", "zsh")
        .with((10, 5), T, "codex")
        .with((5, 1), "Fri Nov 6 01:00:00 2026", "Terminal")
        .with((1, 0), "Thu Nov 5 00:00:00 2026", "launchd")
}

fn dir(tag: &str) -> crate::store::test_dir::TestDir {
    crate::store::test_dir::TestDir::new(&format!("codexrec-{tag}"))
}

fn facts() -> SessionFacts<'static> {
    SessionFacts {
        session_id: "sess-A",
        cwd: "/w/x",
        name: Some("codex-x"),
    }
}

fn record_text(pid: u32, (session, start, domain): (&str, &str, &str)) -> String {
    format!(
        r#"{{"pid":{pid},"sessionId":"{session}","cwd":"/w","procStart":"{start}","pidDomain":"{domain}"}}"#
    )
}

fn put(d: &Path, pid: u32, text: &str) {
    std::fs::write(d.join(format!("{pid}.json")), text).unwrap();
}

fn reg(d: &Path) -> FsRegistry {
    FsRegistry {
        dir: d.to_path_buf(),
    }
}

fn verified_in(d: &Path, t: &dyn ProcessTable, session: &str) -> bool {
    verified_session(30, session, (t, &reg(d), D))
}

#[test]
fn the_program_must_be_exactly_codex() {
    assert!(is_codex("codex"));
    assert!(is_codex("/opt/x/codex"));
    assert!(is_codex("/Users/x y/.local/bin/codex"));
    for no in [
        "",
        "claude",
        "codex-helper",
        "notcodex",
        "codex bg-spare",
        "codex x",
        "node",
        "/tmp/codex x",
        "zsh codex",
    ] {
        assert!(!is_codex(no), "{no:?}");
    }
}

#[test]
fn the_record_goes_under_klatchs_home_and_never_under_a_codex_directory() {
    let p = codex_registry_dir(Path::new("/h/.klatch"));
    assert_eq!(p, Path::new("/h/.klatch/harness/codex"));
    assert!(!p.to_string_lossy().contains(".codex"));
}

#[test]
fn a_hook_under_a_live_codex_process_writes_a_record_of_the_claude_shape() {
    let d = dir("write");
    let r = d.join("reg");
    assert_eq!(write_session_record(&r, 30, &facts(), (&tree(), D)), Ok(10));
    let text = std::fs::read_to_string(r.join("10.json")).unwrap();
    let rec = parse_record(&text).expect("a record the Claude reader parses");
    assert_eq!(
        (rec.pid, rec.session_id.as_str(), rec.cwd.as_str()),
        (10, "sess-A", "/w/x")
    );
    assert_eq!(
        (
            rec.proc_start.as_str(),
            rec.pid_domain.as_str(),
            rec.name.as_deref()
        ),
        (T, D, Some("codex-x"))
    );
    assert_eq!(
        std::fs::metadata(r.join("10.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(&r).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::read_dir(&r).unwrap().count(),
        1,
        "no temporary file left"
    );
}

#[test]
fn the_nearest_codex_ancestor_is_the_one_recorded() {
    let d = dir("nearest");
    let t = tree()
        .with((20, 15), "Sat Nov 7 09:59:59 2026", "zsh")
        .with((15, 10), "Sat Nov 7 09:59:58 2026", "codex");
    assert_eq!(
        write_session_record(&d.join("r"), 30, &facts(), (&t, D)),
        Ok(15)
    );
}

#[test]
fn no_codex_above_the_hook_writes_nothing() {
    let d = dir("none");
    let t = tree().with((10, 5), T, "claude");
    let r = d.join("r");
    assert_eq!(
        write_session_record(&r, 30, &facts(), (&t, D)),
        Err(NotRecorded::NoCodex)
    );
    assert!(!r.exists(), "not even the directory");
}

#[test]
fn an_unreadable_table_writes_nothing_and_says_so() {
    let d = dir("unreadable");
    let r = d.join("r");
    assert_eq!(
        write_session_record(&r, 30, &facts(), (&UnreadableTable, D)),
        Err(NotRecorded::TableUnreadable)
    );
    assert!(!r.exists());
}

#[test]
fn a_table_that_fails_partway_up_writes_nothing() {
    struct Flaky(FakeTable);
    impl ProcessTable for Flaky {
        fn lookup(&self, pid: u32) -> Lookup {
            if pid == 10 {
                Lookup::Unreadable
            } else {
                self.0.lookup(pid)
            }
        }
    }
    let d = dir("flaky");
    assert_eq!(
        write_session_record(&d.join("r"), 30, &facts(), (&Flaky(tree()), D)),
        Err(NotRecorded::TableUnreadable)
    );
}

#[test]
fn a_parent_loop_ends_instead_of_spinning() {
    let d = dir("loop");
    let t = FakeTable::default()
        .with((2, 3), T, "a")
        .with((3, 2), T, "b");
    assert_eq!(
        write_session_record(&d.join("r"), 2, &facts(), (&t, D)),
        Err(NotRecorded::NoCodex)
    );
}

#[test]
fn a_directory_that_cannot_be_made_is_unwritable_not_a_panic() {
    let d = dir("blocked");
    std::fs::write(d.join("file"), "x").unwrap();
    assert_eq!(
        write_session_record(&d.join("file").join("r"), 30, &facts(), (&tree(), D)),
        Err(NotRecorded::Unwritable)
    );
}

#[test]
fn a_registry_directory_that_is_a_symlink_is_refused() {
    let d = dir("symlink");
    let elsewhere = d.join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, d.join("r")).unwrap();
    assert_eq!(
        write_session_record(&d.join("r"), 30, &facts(), (&tree(), D)),
        Err(NotRecorded::Unwritable)
    );
    assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
}

#[test]
fn a_record_that_is_a_symlink_is_not_followed_on_write() {
    let d = dir("filelink");
    let r = d.join("r");
    std::fs::create_dir(&r).unwrap();
    std::fs::set_permissions(&r, std::fs::Permissions::from_mode(0o700)).unwrap();
    let target = d.join("victim");
    std::fs::write(&target, "keep").unwrap();
    std::os::unix::fs::symlink(&target, r.join("10.json")).unwrap();
    let _ = write_session_record(&r, 30, &facts(), (&tree(), D));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
}

#[test]
fn writing_removes_records_of_processes_that_are_gone_and_keeps_the_rest() {
    let d = dir("prune");
    let r = d.join("r");
    std::fs::create_dir(&r).unwrap();
    std::fs::set_permissions(&r, std::fs::Permissions::from_mode(0o700)).unwrap();
    put(&r, 99, &record_text(99, ("dead", T, D))); // no such process
    put(&r, 5, &record_text(5, ("reused", T, D))); // pid exists, other start time
    let other = tree().with((77, 1), "Sat Nov 7 08:00:00 2026", "codex");
    put(
        &r,
        77,
        &record_text(77, ("live-other", "Sat Nov 7 08:00:00 2026", D)),
    );
    std::fs::write(r.join("notes.txt"), "mine").unwrap();
    write_session_record(&r, 30, &facts(), (&other, D)).unwrap();
    assert!(!r.join("99.json").exists() && !r.join("5.json").exists());
    assert!(r.join("77.json").exists() && r.join("10.json").exists());
    assert!(r.join("notes.txt").exists());
}

#[test]
fn a_record_from_an_unreadable_table_or_another_machine_is_never_pruned() {
    let d = dir("keep");
    let r = d.join("r");
    std::fs::create_dir(&r).unwrap();
    std::fs::set_permissions(&r, std::fs::Permissions::from_mode(0o700)).unwrap();
    put(&r, 88, &record_text(88, ("elsewhere", T, "linux")));
    write_session_record(&r, 30, &facts(), (&tree(), D)).unwrap();
    assert!(r.join("88.json").exists());
}

#[test]
fn a_live_record_above_the_caller_is_verified_for_its_own_session() {
    let d = dir("live");
    put(&d, 10, &record_text(10, ("sess-A", T, D)));
    assert!(verified_in(&d, &tree(), "sess-A"));
}

#[test]
fn a_record_of_another_session_is_not_verified() {
    let d = dir("othersession");
    put(&d, 10, &record_text(10, ("sess-B", T, D)));
    assert!(!verified_in(&d, &tree(), "sess-A"));
}

#[test]
fn a_dead_process_is_not_verified() {
    let d = dir("dead");
    put(&d, 10, &record_text(10, ("sess-A", T, D)));
    let t = tree();
    let mut gone = FakeTable::default();
    gone.0 =
        t.0.iter()
            .filter(|(p, _)| **p != 10)
            .map(|(p, i)| (*p, i.clone()))
            .collect();
    let gone = gone.with((20, 5), "Sat Nov 7 09:59:59 2026", "zsh");
    assert!(!verified_in(&d, &gone, "sess-A"));
}

#[test]
fn a_reused_pid_is_not_verified() {
    let d = dir("reused");
    put(
        &d,
        10,
        &record_text(10, ("sess-A", "Mon Nov 2 08:00:00 2026", D)),
    );
    assert!(!verified_in(&d, &tree(), "sess-A"));
}

#[test]
fn the_right_pid_and_start_running_another_program_is_not_verified() {
    let d = dir("program");
    put(&d, 10, &record_text(10, ("sess-A", T, D)));
    for program in ["claude", "bash", "codex-helper", "node"] {
        let t = tree().with((10, 5), T, program);
        assert!(!verified_in(&d, &t, "sess-A"), "{program}");
    }
}

#[test]
fn a_record_of_another_pid_domain_is_not_verified() {
    let d = dir("domain");
    put(&d, 10, &record_text(10, ("sess-A", T, "linux")));
    assert!(!verified_in(&d, &tree(), "sess-A"));
}

#[test]
fn a_record_naming_a_process_that_is_not_above_the_caller_is_not_verified() {
    // another user's (or another session's) live codex process, with a perfect record, beside the caller's tree
    let d = dir("beside");
    let t = tree().with((50, 1), T, "codex");
    put(&d, 50, &record_text(50, ("sess-A", T, D)));
    assert!(!verified_in(&d, &t, "sess-A"));
}

#[test]
fn a_record_filed_under_another_pid_is_not_believed() {
    let d = dir("misfiled");
    put(&d, 10, &record_text(11, ("sess-A", T, D)));
    assert!(!verified_in(&d, &tree(), "sess-A"));
}

#[test]
fn a_malformed_record_a_missing_registry_and_an_unreadable_table_are_not_verified() {
    let d = dir("broken");
    put(&d, 10, "not json");
    assert!(!verified_in(&d, &tree(), "sess-A"));
    assert!(!verified_in(&d.join("nope"), &tree(), "sess-A"));
    put(&d, 10, &record_text(10, ("sess-A", T, D)));
    assert!(!verified_session(
        30,
        "sess-A",
        (&UnreadableTable, &reg(&d), D)
    ));
}

#[test]
fn a_claude_record_in_the_claude_registry_is_not_a_codex_record() {
    // the Codex walk reads only the directory it is given, and a claude process never passes the program test
    let d = dir("claudeproc");
    put(&d, 10, &record_text(10, ("sess-A", T, D)));
    let t = tree().with((10, 5), T, "claude");
    assert!(!verified_in(&d, &t, "sess-A"));
}

#[test]
fn a_forged_record_for_the_real_codex_ancestor_is_believed_as_the_claude_registry_believes_it() {
    // the documented grade: same-user evidence. The forger needs a live codex ancestor at that pid and start time.
    let d = dir("forged");
    put(&d, 10, &record_text(10, ("sess-A", T, D)));
    assert!(verified_in(&d, &tree(), "sess-A"));
    // but not for a pid of its own invention, and not with a guessed start time
    let d2 = dir("forged2");
    put(
        &d2,
        10,
        &record_text(10, ("sess-A", "Sun Nov 8 00:00:00 2026", D)),
    );
    put(
        &d2,
        20,
        &record_text(20, ("sess-A", "Sat Nov 7 09:59:59 2026", D)),
    );
    assert!(!verified_in(&d2, &tree(), "sess-A"), "a shell is not codex");
}

fn own(d: &Path, uid: u32) -> OwnRegistry {
    OwnRegistry {
        dir: d.to_path_buf(),
        uid,
    }
}

pub(super) fn me() -> u32 {
    unsafe { libc::geteuid() }
}

#[test]
fn an_owned_registry_reads_a_plain_file_of_its_own_user() {
    let d = dir("own-ok");
    put(&d, 10, &record_text(10, ("sess-A", T, D)));
    assert!(own(&d, me()).record_for(10).is_some());
    assert!(verified_session(30, "sess-A", (&tree(), &own(&d, me()), D)));
}

#[test]
fn a_record_owned_by_another_user_is_not_read() {
    let d = dir("own-foreign");
    put(&d, 10, &record_text(10, ("sess-A", T, D)));
    assert!(own(&d, me().wrapping_add(1)).record_for(10).is_none());
}

#[test]
fn a_record_others_can_write_is_not_read() {
    let d = dir("own-open");
    put(&d, 10, &record_text(10, ("sess-A", T, D)));
    for mode in [0o666, 0o620, 0o602] {
        std::fs::set_permissions(d.join("10.json"), std::fs::Permissions::from_mode(mode)).unwrap();
        assert!(own(&d, me()).record_for(10).is_none(), "{mode:o}");
    }
}

#[test]
fn a_record_that_is_a_symlink_is_not_read() {
    let d = dir("own-link");
    std::fs::write(d.join("real"), record_text(10, ("sess-A", T, D))).unwrap();
    std::os::unix::fs::symlink(d.join("real"), d.join("10.json")).unwrap();
    assert!(own(&d, me()).record_for(10).is_none());
}

#[test]
fn the_name_is_codex_only_for_a_verified_session_and_no_level_ever_changes() {
    for verified in [true, false] {
        let h = CodexRecorded::new(Box::new(move || verified));
        assert_eq!(h.id(), if verified { "codex" } else { "generic" });
        for c in Capability::ALL {
            assert_eq!(h.level(c), crate::harness::Generic.level(c), "{c:?}");
            assert_eq!(h.basis(c), crate::harness::Generic.basis(c));
        }
        assert_ne!(h.level(Capability::HeadlessWorker), Level::Native);
        assert_ne!(h.level(Capability::PreActionEnforcement), Level::Native);
    }
}

#[test]
fn the_check_runs_once_and_only_when_asked() {
    use std::cell::Cell;
    use std::rc::Rc;
    let calls = Rc::new(Cell::new(0));
    let seen = calls.clone();
    let h = CodexRecorded::new(Box::new(move || {
        seen.set(seen.get() + 1);
        true
    }));
    assert_eq!(h.level(Capability::Wake), Level::Absent);
    assert_eq!(calls.get(), 0, "levels never need the check");
    assert_eq!((h.id(), h.id()), ("codex", "codex"));
    assert_eq!(calls.get(), 1);
}

#[test]
fn a_registry_directory_of_another_user_is_refused() {
    let d = dir("foreign-dir");
    let r = d.join("r");
    std::fs::create_dir(&r).unwrap();
    assert_eq!(safe_dir(&r, me()), Ok(()));
    assert_eq!(
        safe_dir(&r, me().wrapping_add(1)),
        Err(NotRecorded::Unwritable)
    );
}

#[test]
fn a_record_that_cannot_be_put_in_place_leaves_no_temporary_file() {
    let d = dir("rename-fails");
    let r = d.join("r");
    std::fs::create_dir_all(r.join("10.json")).unwrap(); // a directory where the record goes
    std::fs::set_permissions(&r, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        write_session_record(&r, 30, &facts(), (&tree(), D)),
        Err(NotRecorded::Unwritable)
    );
    let names: Vec<_> = std::fs::read_dir(&r)
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(
        names,
        vec![std::ffi::OsString::from("10.json")],
        "{names:?}"
    );
}
