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
