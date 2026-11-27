use super::*;
use std::time::Duration;

fn dir(tag: &str) -> crate::store::test_dir::TestDir {
    crate::store::test_dir::TestDir::new(&format!("ownrec-{tag}"))
}

fn me() -> u32 {
    unsafe { libc::geteuid() }
}

fn mode(p: &Path, m: u32) {
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap();
}

const REC: &str = r#"{"pid":10,"sessionId":"s","cwd":"/w","procStart":"x","pidDomain":"darwin"}"#;

#[test]
fn a_plain_sealed_file_of_ours_is_read_with_mode_0600_or_0644() {
    let d = dir("ok");
    mode(&d, 0o700);
    for m in [0o600, 0o644, 0o400] {
        std::fs::write(d.join("10.json"), REC).unwrap();
        mode(&d.join("10.json"), m);
        assert_eq!(
            read_named(&d, "10.json", me()).as_deref(),
            Some(REC),
            "{m:o}"
        );
    }
}

#[test]
fn a_file_group_or_others_can_write_is_not_read() {
    let d = dir("open");
    mode(&d, 0o700);
    std::fs::write(d.join("10.json"), REC).unwrap();
    for m in [0o666, 0o620, 0o602] {
        mode(&d.join("10.json"), m);
        assert_eq!(read_named(&d, "10.json", me()), None, "{m:o}");
    }
}

#[test]
fn a_file_of_another_user_is_not_read() {
    let d = dir("foreign");
    mode(&d, 0o700);
    std::fs::write(d.join("10.json"), REC).unwrap();
    assert_eq!(read_named(&d, "10.json", me().wrapping_add(1)), None);
}

#[test]
fn a_symlink_a_fifo_and_a_directory_are_never_read() {
    let d = dir("kinds");
    mode(&d, 0o700);
    std::fs::write(d.join("real"), REC).unwrap();
    std::os::unix::fs::symlink(d.join("real"), d.join("link")).unwrap();
    let c = std::ffi::CString::new(d.join("fifo").to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    std::fs::create_dir(d.join("sub")).unwrap();
    for name in ["link", "sub", "missing"] {
        assert_eq!(read_named(&d, name, me()), None, "{name}");
    }
    // a FIFO is refused without waiting for a writer: bounded, and released if the open did block
    let (path, dir) = (d.join("fifo"), d.to_path_buf());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(read_named(&dir, "fifo", me()));
    });
    let got = rx.recv_timeout(Duration::from_secs(10));
    if got.is_err() {
        let _ = std::fs::OpenOptions::new().write(true).open(&path);
    }
    assert_eq!(got, Ok(None), "a FIFO stalled the open");
}

#[test]
fn the_cap_is_exact() {
    let d = dir("cap");
    mode(&d, 0o700);
    let at = "x".repeat(MAX_RECORD_BYTES as usize);
    std::fs::write(d.join("at"), &at).unwrap();
    std::fs::write(d.join("over"), format!("{at}x")).unwrap();
    assert_eq!(read_named(&d, "at", me()).map(|t| t.len()), Some(at.len()));
    assert_eq!(read_named(&d, "over", me()), None);
}

#[test]
fn the_check_and_the_read_use_one_descriptor_so_a_swap_in_between_is_not_believed() {
    let d = dir("swap");
    mode(&d, 0o700);
    std::fs::write(d.join("10.json"), REC).unwrap();
    let file = open_plain(&d.join("10.json")).unwrap();
    // after the open, the name is replaced by a symlink to something else
    std::fs::remove_file(d.join("10.json")).unwrap();
    std::fs::write(d.join("other"), "swapped").unwrap();
    std::os::unix::fs::symlink(d.join("other"), d.join("10.json")).unwrap();
    assert_eq!(read_plain(file, me()).as_deref(), Some(REC));
    assert_eq!(
        read_named(&d, "10.json", me()),
        None,
        "the symlink is refused"
    );
}

#[test]
fn a_registry_directory_others_can_write_is_refused_for_every_read() {
    let d = dir("open-dir");
    std::fs::write(d.join("10.json"), REC).unwrap();
    mode(&d, 0o700);
    let reg = OwnRegistry {
        dir: d.to_path_buf(),
        uid: me(),
    };
    assert!(reg.record_for(10).is_some());
    assert_eq!(reg.records().0.len(), 1);
    for m in [0o777, 0o770, 0o707] {
        mode(&d, m);
        assert!(reg.record_for(10).is_none(), "{m:o}");
        assert_eq!(reg.records(), (Vec::new(), 0), "{m:o}");
    }
    mode(&d, 0o700);
}

#[test]
fn a_registry_directory_that_is_a_symlink_or_another_users_is_refused() {
    let d = dir("dirkinds");
    mode(&d, 0o700);
    std::fs::create_dir(d.join("real")).unwrap();
    mode(&d.join("real"), 0o700);
    std::fs::write(d.join("real").join("10.json"), REC).unwrap();
    std::os::unix::fs::symlink(d.join("real"), d.join("link")).unwrap();
    let own = |dir: PathBuf, uid| OwnRegistry { dir, uid };
    assert!(own(d.join("real"), me()).record_for(10).is_some());
    assert!(own(d.join("link"), me()).record_for(10).is_none());
    assert!(
        own(d.join("real"), me().wrapping_add(1))
            .record_for(10)
            .is_none()
    );
}

#[test]
fn records_lists_good_ones_counts_the_bad_and_skips_what_is_not_a_record_name() {
    let d = dir("list");
    mode(&d, 0o700);
    std::fs::write(d.join("10.json"), REC).unwrap();
    std::fs::write(d.join("11.json"), REC).unwrap(); // misfiled
    std::fs::write(d.join("12.json"), "not json").unwrap();
    std::fs::write(d.join("notes.txt"), REC).unwrap();
    std::fs::write(d.join("13.abc.key"), "secret").unwrap();
    let reg = OwnRegistry {
        dir: d.to_path_buf(),
        uid: me(),
    };
    let (records, malformed) = reg.records();
    assert_eq!((records.len(), malformed), (1, 2));
}

#[test]
fn only_the_first_entries_are_examined_in_one_pass() {
    let d = dir("many");
    mode(&d, 0o700);
    for pid in 100..100 + MAX_ENTRIES + 50 {
        std::fs::write(
            d.join(format!("{pid}.json")),
            REC.replace("\"pid\":10", &format!("\"pid\":{pid}")),
        )
        .unwrap();
    }
    let reg = OwnRegistry {
        dir: d.to_path_buf(),
        uid: me(),
    };
    assert_eq!(reg.records().0.len(), MAX_ENTRIES);
}

fn age(p: &Path, secs: u64) {
    let when = std::time::SystemTime::now() - Duration::from_secs(secs);
    std::fs::File::options()
        .write(true)
        .open(p)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

#[test]
fn old_temporary_files_are_removed_and_young_and_other_files_are_kept() {
    let d = dir("temps");
    mode(&d, 0o700);
    for name in [
        ".10.tmp1", ".10.tmp2", ".11.tmp9", "10.json", "notes", ".x.tmp1", ".10.tmp",
    ] {
        std::fs::write(d.join(name), "x").unwrap();
    }
    age(&d.join(".10.tmp1"), 120);
    age(&d.join(".x.tmp1"), 120);
    age(&d.join("notes"), 120);
    age(&d.join(".10.tmp"), 120);
    remove_stale_temps(&d, Duration::from_secs(60));
    assert!(!d.join(".10.tmp1").exists(), "old temporary file");
    for kept in [
        ".10.tmp2", ".11.tmp9", "10.json", "notes", ".x.tmp1", ".10.tmp",
    ] {
        assert!(d.join(kept).exists(), "{kept}");
    }
}

#[test]
fn a_temporary_name_that_is_a_symlink_or_a_directory_is_not_removed() {
    let d = dir("temps-kinds");
    mode(&d, 0o700);
    std::fs::write(d.join("victim"), "keep").unwrap();
    std::os::unix::fs::symlink(d.join("victim"), d.join(".10.tmp1")).unwrap();
    std::fs::create_dir(d.join(".10.tmp2")).unwrap();
    remove_stale_temps(&d, Duration::ZERO);
    assert_eq!(std::fs::read_to_string(d.join("victim")).unwrap(), "keep");
    assert!(d.join(".10.tmp1").symlink_metadata().is_ok() && d.join(".10.tmp2").is_dir());
}

#[test]
fn a_source_that_holds_more_than_the_cap_is_refused_whatever_its_size_said() {
    assert_eq!(
        read_capped(std::io::repeat(b'x').take(20), 10),
        None,
        "longer than the cap"
    );
    assert_eq!(
        read_capped(&b"0123456789"[..], 10).as_deref(),
        Some("0123456789")
    );
    assert_eq!(read_capped(&b"01234567890"[..], 10), None, "one over");
    assert_eq!(read_capped(&[0xff, 0xfe][..], 10), None, "not text");
}

#[test]
fn no_more_than_one_byte_past_the_cap_is_ever_read() {
    struct Counting(std::rc::Rc<std::cell::Cell<usize>>);
    impl Read for Counting {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min((1 << 20) - self.0.get().min(1 << 20));
            buf[..n].fill(b'x');
            self.0.set(self.0.get() + n);
            Ok(n)
        }
    }
    let served = std::rc::Rc::new(std::cell::Cell::new(0));
    assert_eq!(read_capped(Counting(served.clone()), 10), None);
    assert!(served.get() <= 11, "read {} bytes", served.get());
}

fn record_text(pid: usize) -> String {
    REC.replace("\"pid\":10", &format!("\"pid\":{pid}"))
}

fn own(d: &Path) -> OwnRegistry {
    OwnRegistry {
        dir: d.to_path_buf(),
        uid: me(),
    }
}

#[test]
fn junk_entries_do_not_use_up_the_entry_bound_of_a_listing() {
    let d = dir("junk-list");
    mode(&d, 0o700);
    for i in 0..3000 {
        std::fs::write(d.join(format!("junk-{i}.txt")), "x").unwrap();
    }
    for pid in 100..120 {
        std::fs::write(d.join(format!("{pid}.json")), record_text(pid)).unwrap();
    }
    assert_eq!(
        own(&d).records().0.len(),
        20,
        "a record was starved by junk"
    );
}

#[test]
fn junk_entries_do_not_use_up_the_entry_bound_of_the_temp_cleanup() {
    let d = dir("junk-temps");
    mode(&d, 0o700);
    for i in 0..3000 {
        std::fs::write(d.join(format!("junk-{i}.txt")), "x").unwrap();
    }
    for n in 0..5 {
        let temp = d.join(format!(".10.tmp{n}"));
        std::fs::write(&temp, "x").unwrap();
        age(&temp, 120);
    }
    remove_stale_temps(&d, Duration::from_secs(60));
    for n in 0..5 {
        assert!(!d.join(format!(".10.tmp{n}")).exists(), "temp {n} left");
    }
}

#[test]
fn the_temp_cleanup_looks_at_no_more_than_the_entry_bound_of_temporary_files() {
    let d = dir("temps-bound");
    mode(&d, 0o700);
    let n = MAX_ENTRIES + 20;
    for i in 0..n {
        let temp = d.join(format!(".10.tmp{i}"));
        std::fs::write(&temp, "x").unwrap();
        age(&temp, 120);
    }
    remove_stale_temps(&d, Duration::from_secs(60));
    let left = (0..n)
        .filter(|i| d.join(format!(".10.tmp{i}")).exists())
        .count();
    assert_eq!(left, 20, "exactly the bound is removed in one pass");
}

/// A shuffled but fixed age for each pid, so directory order has nothing to do with age.
fn shuffled_age(pid: usize, n: usize) -> usize {
    (pid * 7919) % n
}

#[test]
fn with_more_records_than_the_bound_the_newest_are_read_not_the_first_in_directory_order() {
    let d = dir("newest");
    mode(&d, 0o700);
    let n = MAX_ENTRIES + 44;
    for i in 0..n {
        let file = d.join(format!("{}.json", 1000 + i));
        std::fs::write(&file, record_text(1000 + i)).unwrap();
        // rank 0 is the oldest; ages are whole minutes apart, so no tie
        age(&file, 100_000 - 60 * shuffled_age(i, n) as u64);
    }
    let mut expect: Vec<usize> = (0..n)
        .filter(|&i| shuffled_age(i, n) >= n - MAX_ENTRIES)
        .collect();
    expect.sort_unstable();
    let expect: Vec<u32> = expect.into_iter().map(|i| 1000 + i as u32).collect();
    let got: Vec<u32> = own(&d).records().0.iter().map(|r| r.pid).collect();
    assert_eq!(
        got, expect,
        "a live (newest) record was hidden by older ones"
    );
}

fn set_mtime(p: &Path, when: std::time::SystemTime) {
    std::fs::File::options()
        .write(true)
        .open(p)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

#[test]
fn junk_with_a_future_mtime_does_not_hide_a_valid_record() {
    let d = dir("future-junk");
    mode(&d, 0o700);
    std::fs::write(d.join("5000.json"), record_text(5000)).unwrap();
    let year = std::time::SystemTime::now() + Duration::from_secs(365 * 86_400);
    for i in 0..MAX_ENTRIES {
        let f = d.join(format!("{i}.json"));
        std::fs::write(&f, "junk").unwrap();
        set_mtime(&f, year);
    }
    let (records, malformed) = own(&d).records();
    assert_eq!(records.iter().map(|r| r.pid).collect::<Vec<_>>(), [5000]);
    assert_eq!(malformed, MAX_ENTRIES);
}

#[test]
fn fresh_junk_does_not_hide_an_older_valid_record() {
    let d = dir("fresh-junk");
    mode(&d, 0o700);
    let old = d.join("5000.json");
    std::fs::write(&old, record_text(5000)).unwrap();
    set_mtime(
        &old,
        std::time::SystemTime::now() - Duration::from_secs(30 * 86_400),
    );
    for i in 0..MAX_ENTRIES + 44 {
        std::fs::write(d.join(format!("{i}.json")), "junk").unwrap();
    }
    assert_eq!(own(&d).records().0.len(), 1);
}

#[test]
fn a_listing_looks_at_no_more_than_the_scan_bound_of_entries() {
    let d = dir("scan-bound");
    mode(&d, 0o700);
    for pid in 100..110 {
        std::fs::write(d.join(format!("{pid}.json")), record_text(pid)).unwrap();
    }
    let read = |n: &str| read_named(&d, n, me());
    let (records, malformed) = scan_records_with(&d, 3, 256, 4096, read).unwrap();
    assert_eq!(records.len() + malformed, 3);
    let (records, _) = scan_records_with(&d, 10, 256, 4096, read).unwrap();
    assert_eq!(records.len(), 10);
}

#[test]
fn a_listing_opens_no_more_than_the_open_bound_of_files() {
    let d = dir("open-bound");
    mode(&d, 0o700);
    for i in 0..20 {
        std::fs::write(d.join(format!("{i}.json")), "junk").unwrap();
    }
    let read = |n: &str| read_named(&d, n, me());
    assert_eq!(
        scan_records_with(&d, 100, 256, 5, read).unwrap(),
        (vec![], 5)
    );
}

/// Swap the name between a plain file and a symlink to another plain file of ours, as fast as it can, until told to stop.
fn swapper(
    dir: PathBuf,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> std::thread::JoinHandle<()> {
    use std::sync::atomic::Ordering;
    std::thread::spawn(move || {
        let mut i = 0u64;
        while !stop.load(Ordering::Relaxed) {
            i += 1;
            let stage = dir.join(format!("stage{}", i % 2));
            let _ = std::fs::remove_file(&stage);
            if i.is_multiple_of(2) {
                let _ = std::os::unix::fs::symlink(dir.join("other"), &stage);
            } else {
                let _ = std::fs::hard_link(dir.join("real"), &stage);
            }
            let _ = std::fs::rename(&stage, dir.join("10.json"));
        }
    })
}

#[test]
fn a_name_swapped_to_a_symlink_while_it_is_read_never_gives_the_text_behind_the_link() {
    use std::sync::atomic::Ordering;
    let d = dir("race");
    mode(&d, 0o700);
    std::fs::write(d.join("real"), REC).unwrap();
    std::fs::write(d.join("other"), "swapped").unwrap();
    std::fs::hard_link(d.join("real"), d.join("10.json")).unwrap();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let thread = swapper(d.to_path_buf(), stop.clone());
    let registry = own(&d);
    let started = std::time::Instant::now();
    let (mut reads, mut leaked) = (0u32, 0u32);
    while started.elapsed() < Duration::from_secs(3) && leaked == 0 {
        for _ in 0..200 {
            reads += 1;
            // through the named reader and through the registry, the two ways a record is opened
            if read_named(&d, "10.json", me()).as_deref() == Some("swapped")
                || registry
                    .record_for(10)
                    .is_some_and(|r| r.session_id == "swapped")
                || registry
                    .records()
                    .0
                    .iter()
                    .any(|r| r.session_id == "swapped")
            {
                leaked += 1;
            }
        }
    }
    stop.store(true, Ordering::Relaxed);
    thread.join().unwrap();
    assert_eq!(
        leaked, 0,
        "the text behind a swapped-in symlink was believed ({reads} reads)"
    );
}

#[test]
fn a_fifo_as_the_recovery_attempts_file_never_blocks_the_count_or_the_charge() {
    // (here because the unsafe hygiene allowlist covers this directory; the code under test is proof::filelimit, #1636)
    let d = dir("recovery-fifo");
    let db = d.join("rooms.db");
    let file = crate::proof::recovery_attempts_file(&db);
    let c = std::ffi::CString::new(file.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    let asked = db.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(crate::proof::recovery_attempts_recorded(&asked));
    });
    let got = rx.recv_timeout(Duration::from_secs(10));
    if got.is_err() {
        let _ = std::fs::OpenOptions::new().write(true).open(&file);
    }
    assert_eq!(got, Ok(u32::MAX), "a FIFO stalled the open");
}
