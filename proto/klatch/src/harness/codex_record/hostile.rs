//! Entries in the registry directory that are not plain records must never stop a hook, exhaust memory or be read
//! (#884 review): a FIFO, a symlink to a huge file, an oversized file, a directory named like a record.
use super::*;
use crate::harness::liveness::fake::FakeTable;
use std::os::unix::fs::PermissionsExt;
use std::sync::mpsc;
use std::time::Duration;

const T: &str = "Fri Nov 6 03:04:05 2026";
const D: &str = "darwin";
const BOUND: Duration = Duration::from_secs(20);

fn tree() -> FakeTable {
    FakeTable::default()
        .with((30, 20), "Sat Nov 7 10:00:00 2026", "klatch")
        .with((20, 10), "Sat Nov 7 09:59:59 2026", "zsh")
        .with((10, 5), T, "codex")
        .with((5, 1), "Fri Nov 6 01:00:00 2026", "Terminal")
}

fn registry(tag: &str) -> (crate::store::test_dir::TestDir, PathBuf) {
    let d = crate::store::test_dir::TestDir::new(&format!("codexhostile-{tag}"));
    let r = d.join("r");
    std::fs::create_dir(&r).unwrap();
    std::fs::set_permissions(&r, std::fs::Permissions::from_mode(0o700)).unwrap();
    (d, r)
}

fn mkfifo(path: &Path) {
    let c = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
}

fn dead_record(r: &Path, pid: u32) {
    std::fs::write(
        r.join(format!("{pid}.json")),
        format!(
            r#"{{"pid":{pid},"sessionId":"old","cwd":"/w","procStart":"{T}","pidDomain":"{D}"}}"#
        ),
    )
    .unwrap();
}

/// Run `f` on a thread with a bound. `None` when it did not finish; a FIFO it may be stuck on is then opened for
/// writing, which releases it, so no thread or process is left behind.
fn bounded<T: Send + 'static>(
    fifo: Option<&Path>,
    f: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    let got = rx.recv_timeout(BOUND).ok();
    if got.is_none()
        && let Some(f) = fifo
    {
        let _ = std::fs::OpenOptions::new().write(true).open(f);
    }
    got
}

fn write_in(r: &Path) -> Option<Result<u32, NotRecorded>> {
    let r = r.to_path_buf();
    bounded(Some(&r.join("999.json")), move || {
        let facts = SessionFacts {
            session_id: "sess-A",
            cwd: "/w",
            name: None,
        };
        write_session_record(&r, 30, &facts, (&tree(), D))
    })
}

#[test]
fn a_fifo_named_like_a_record_does_not_hang_the_write() {
    let (_d, r) = registry("fifo");
    mkfifo(&r.join("999.json"));
    assert_eq!(write_in(&r), Some(Ok(10)), "the hook hung on a FIFO");
    assert!(r.join("10.json").exists());
}

#[test]
fn a_symlink_to_an_enormous_file_is_not_read() {
    let (d, r) = registry("sparse");
    let big = d.join("big");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(16 << 20)
        .unwrap();
    std::os::unix::fs::symlink(&big, r.join("998.json")).unwrap();
    assert_eq!(write_in(&r), Some(Ok(10)));
    assert!(r.join("998.json").exists(), "left alone");
}

#[test]
fn an_oversized_record_is_not_read_and_not_removed() {
    let (_d, r) = registry("oversized");
    std::fs::write(r.join("997.json"), vec![b'x'; 200 << 10]).unwrap();
    assert_eq!(write_in(&r), Some(Ok(10)));
    assert!(r.join("997.json").exists());
}

#[test]
fn a_directory_named_like_a_record_is_skipped() {
    let (_d, r) = registry("dirname");
    std::fs::create_dir(r.join("996.json")).unwrap();
    assert_eq!(write_in(&r), Some(Ok(10)));
    assert!(r.join("996.json").is_dir());
}

#[test]
fn the_other_records_are_still_processed_beside_hostile_entries() {
    let (d, r) = registry("mixed");
    mkfifo(&r.join("999.json"));
    let big = d.join("big");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(16 << 20)
        .unwrap();
    std::os::unix::fs::symlink(&big, r.join("998.json")).unwrap();
    std::fs::write(r.join("997.json"), vec![b'x'; 200 << 10]).unwrap();
    std::fs::create_dir(r.join("996.json")).unwrap();
    dead_record(&r, 995);
    assert_eq!(write_in(&r), Some(Ok(10)));
    assert!(!r.join("995.json").exists(), "the dead record was pruned");
    assert!(r.join("10.json").exists());
}

#[test]
fn a_write_removes_old_temporary_files_and_keeps_young_ones() {
    let (_d, r) = registry("temps");
    std::fs::write(r.join(".77.tmp1"), "x").unwrap();
    std::fs::write(r.join(".77.tmp2"), "x").unwrap();
    let old = std::time::SystemTime::now() - Duration::from_secs(120);
    std::fs::File::options()
        .write(true)
        .open(r.join(".77.tmp1"))
        .unwrap()
        .set_modified(old)
        .unwrap();
    assert_eq!(write_in(&r), Some(Ok(10)));
    assert!(!r.join(".77.tmp1").exists(), "a killed writer's leftover");
    assert!(r.join(".77.tmp2").exists(), "a writer may be at work");
}
