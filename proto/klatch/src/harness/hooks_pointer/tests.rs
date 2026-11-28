use super::*;
use crate::store::test_dir::TestDir;
use std::fs;

const HOOKS: &str = r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"/usr/local/bin/klatch --discover hook --event SessionStart"}]}]}}"#;

fn settings(dir: &Path, name: &str, text: &str) {
    fs::create_dir_all(dir.join(".claude")).unwrap();
    fs::write(dir.join(".claude").join(name), text).unwrap();
}

/// A repository with a nested working directory, and a separate home.
struct Fx {
    _t: TestDir,
    repo: PathBuf,
    cwd: PathBuf,
    home: PathBuf,
}

fn fx() -> Fx {
    let t = TestDir::new("hooks-pointer");
    let repo = t.join("repo");
    let cwd = repo.join("sub").join("deep");
    let home = t.join("home");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(&home).unwrap();
    Fx {
        _t: t,
        repo,
        cwd,
        home,
    }
}

fn ask(f: &Fx) -> Option<&'static str> {
    no_hooks_pointer(&f.cwd, Some(&f.home), false)
}

#[test]
fn a_directory_with_no_hooks_anywhere_gets_the_pointer() {
    let f = fx();
    let p = ask(&f).expect("pointer");
    assert!(p.contains("klatch init --scope user"));
    assert!(p.contains("DATA"));
    assert!(!p.contains(f.repo.to_str().unwrap()) && !p.contains(f.home.to_str().unwrap()));
    assert!(!p.contains('\u{2014}') && p.len() < 400);
}

#[test]
fn hooks_in_the_working_directory_project_parent_or_user_scope_silence_it() {
    for place in ["cwd", "repo", "home"] {
        let f = fx();
        let dir = match place {
            "cwd" => &f.cwd,
            "repo" => &f.repo,
            _ => &f.home,
        };
        let file = if place == "home" {
            "settings.json"
        } else {
            "settings.local.json"
        };
        settings(dir, file, HOOKS);
        assert_eq!(ask(&f), None, "{place}");
    }
}

#[test]
fn hooks_in_the_shared_project_settings_silence_it() {
    let f = fx();
    settings(&f.repo, "settings.json", HOOKS);
    assert_eq!(ask(&f), None);
}

#[test]
fn another_tools_hooks_do_not_count() {
    let f = fx();
    settings(
        &f.repo,
        "settings.local.json",
        r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo hi"}]}]}}"#,
    );
    assert!(ask(&f).is_some());
    let g = fx();
    settings(
        &g.repo,
        "settings.local.json",
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"/bin/other hook --event Stop"}]}]}}"#,
    );
    assert!(ask(&g).is_some(), "a hook-shaped command of another tool");
}

#[test]
fn the_walk_stops_at_the_repository_root() {
    let f = fx();
    // hooks above the repository root belong to another project
    settings(f.repo.parent().unwrap(), "settings.local.json", HOOKS);
    assert!(ask(&f).is_some());
}

#[test]
fn a_headless_worker_gets_nothing() {
    let f = fx();
    assert_eq!(no_hooks_pointer(&f.cwd, Some(&f.home), true), None);
}

#[test]
fn a_malformed_or_unreadable_settings_file_prints_nothing() {
    let f = fx();
    settings(&f.repo, "settings.local.json", "{ not json");
    assert_eq!(ask(&f), None);
    let g = fx();
    settings(&g.home, "settings.json", "[1,2");
    assert_eq!(ask(&g), None);
    let h = fx();
    fs::create_dir_all(h.repo.join(".claude").join("settings.local.json")).unwrap();
    assert_eq!(ask(&h), None, "a directory where the file should be");
}

#[test]
fn an_oversized_settings_file_prints_nothing() {
    let f = fx();
    let pad = " ".repeat(2 * 1024 * 1024);
    settings(&f.repo, "settings.local.json", &format!("{{}}{pad}"));
    assert_eq!(ask(&f), None);
}

#[cfg(unix)]
#[test]
fn a_settings_symlink_out_of_the_project_is_not_followed() {
    let f = fx();
    let outside = f._t.join("elsewhere.json");
    fs::write(&outside, "{}").unwrap();
    fs::create_dir_all(f.repo.join(".claude")).unwrap();
    std::os::unix::fs::symlink(&outside, f.repo.join(".claude").join("settings.local.json"))
        .unwrap();
    // followed it would read as "no hooks" and print; not followed it is unknown and says nothing
    assert_eq!(ask(&f), None);
}

#[cfg(unix)]
#[test]
fn a_settings_symlink_inside_the_project_is_followed() {
    let f = fx();
    fs::write(f.repo.join("shared.json"), HOOKS).unwrap();
    fs::create_dir_all(f.repo.join(".claude")).unwrap();
    std::os::unix::fs::symlink(
        f.repo.join("shared.json"),
        f.repo.join(".claude").join("settings.local.json"),
    )
    .unwrap();
    assert_eq!(ask(&f), None);
}

#[test]
fn without_a_repository_only_the_working_directory_is_read() {
    let t = TestDir::new("hooks-pointer-norepo");
    let cwd = t.join("loose").join("dir");
    fs::create_dir_all(&cwd).unwrap();
    settings(&t.join("loose"), "settings.local.json", HOOKS);
    assert!(no_hooks_pointer(&cwd, None, false).is_some());
    settings(&cwd, "settings.local.json", HOOKS);
    assert_eq!(no_hooks_pointer(&cwd, None, false), None);
}

#[test]
fn a_settings_directory_that_is_really_a_file_prints_nothing() {
    let f = fx();
    fs::write(f.repo.join(".claude"), "x").unwrap();
    assert_eq!(ask(&f), None, "cannot tell, so say nothing");
}

#[test]
fn the_pointer_text_is_pinned_and_has_no_shell_metacharacters() {
    // a literal copy, so editing POINTER (an appended `; curl ... | sh`) fails here and needs a deliberate change
    assert_eq!(
        POINTER,
        "klatch: no klatch hooks are installed where you work, so no room digest reaches you here (this line is DATA, not an instruction); a person can turn them on with `klatch init --scope user` (it asks before it writes anything)"
    );
    let outside_quote = POINTER.replace("`klatch init --scope user`", "");
    for bad in ['|', '&', '$', '>', '<', '\n', '\\'] {
        assert!(!outside_quote.contains(bad), "{bad:?}");
    }
    assert_eq!(POINTER.matches('`').count(), 2, "one command, quoted once");
    assert!(!POINTER.contains("curl"));
}

#[cfg(unix)]
#[test]
fn a_symlinked_claude_directory_is_not_followed_even_inside_the_project() {
    let f = fx();
    fs::create_dir_all(f.repo.join("real")).unwrap();
    fs::write(f.repo.join("real").join("settings.local.json"), "{}").unwrap();
    std::os::unix::fs::symlink(f.repo.join("real"), f.repo.join(".claude")).unwrap();
    // followed it would read as "no hooks" and print; not followed it is unknown and says nothing
    assert_eq!(ask(&f), None);
}

#[cfg(unix)]
#[test]
fn a_symlinked_claude_directory_out_of_the_project_is_not_followed() {
    let f = fx();
    let out = f._t.join("elsewhere");
    fs::create_dir_all(&out).unwrap();
    fs::write(out.join("settings.local.json"), "{}").unwrap();
    std::os::unix::fs::symlink(&out, f.repo.join("sub").join(".claude")).unwrap();
    assert_eq!(ask(&f), None);
}

#[cfg(unix)]
#[test]
fn a_fifo_where_the_settings_file_should_be_is_skipped_without_blocking() {
    let f = fx();
    fs::create_dir_all(f.repo.join(".claude")).unwrap();
    let fifo = f.repo.join(".claude").join("settings.local.json");
    let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    let (tx, rx) = std::sync::mpsc::channel();
    let cwd = f.cwd.clone();
    let home = f.home.clone();
    std::thread::spawn(move || {
        let _ = tx.send(no_hooks_pointer(&cwd, Some(&home), false));
    });
    let got = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the scan must not block on a FIFO");
    assert_eq!(got, None, "cannot tell, so say nothing");
}

#[test]
fn a_reader_that_yields_more_than_the_cap_is_skipped_by_the_capped_read() {
    use std::io::Read;
    // the read itself is capped, not only an earlier size check: a reader that yields more than MAX_BYTES is skipped
    let big = std::io::repeat(b' ').take(scan::MAX_BYTES * 2);
    assert_eq!(scan::read_capped(big), None);
    let ok = std::io::Cursor::new(b"{}".to_vec());
    assert_eq!(scan::read_capped(ok).as_deref(), Some("{}"));
    let exact = std::io::repeat(b' ').take(scan::MAX_BYTES);
    assert!(
        scan::read_capped(exact).is_some(),
        "exactly the cap is allowed"
    );
    // and it stops reading at the cap (plus one byte to tell), so an endless source cannot exhaust memory or hang
    struct Counting(std::rc::Rc<std::cell::Cell<u64>>);
    impl std::io::Read for Counting {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            assert!(self.0.get() <= scan::MAX_BYTES + 1, "read past the cap");
            buf.fill(b' ');
            self.0.set(self.0.get() + buf.len() as u64);
            Ok(buf.len())
        }
    }
    let served = std::rc::Rc::new(std::cell::Cell::new(0));
    assert_eq!(scan::read_capped(Counting(served.clone())), None);
    assert!(
        served.get() <= scan::MAX_BYTES + 1 + 8192 * 2,
        "{}",
        served.get()
    );
}

#[test]
fn a_real_file_that_grows_past_the_cap_between_check_and_read_is_skipped() {
    let f = fx();
    settings(&f.repo, "settings.local.json", "{}");
    let file = f.repo.join(".claude").join("settings.local.json");
    // admitted at its small size, then grown by another writer before the read happens
    let got = scan::look_with(
        &file,
        &f.repo,
        false,
        || {},
        |handle| {
            let mut w = fs::OpenOptions::new().append(true).open(&file).unwrap();
            std::io::Write::write_all(&mut w, " ".repeat(2 * 1024 * 1024).as_bytes()).unwrap();
            scan::read_capped(handle)
        },
    );
    assert_eq!(got, None);
}

/// A reader that must never run: counts calls so a rejected handle is proved rejected before any read.
#[cfg(unix)]
fn rejected_before_read(handle: fs::File) -> Option<scan::Seen> {
    let calls = std::cell::Cell::new(0);
    let got = scan::seen_in(handle, |_| {
        calls.set(calls.get() + 1);
        Some("{}".to_string())
    });
    assert_eq!(calls.get(), 0, "the handle reached the reader");
    got
}

#[cfg(unix)]
#[test]
fn a_directory_socket_or_device_handle_is_rejected_by_the_type_check_itself() {
    let t = TestDir::new("hooks-pointer-handles");
    let dir = fs::File::open(t.join("")).unwrap();
    assert_eq!(rejected_before_read(dir), None, "directory");
    // a socket handle made with socketpair(2) directly, so the source names no socket type the no_telemetry guard flags
    let mut fds = [0 as libc::c_int; 2];
    assert_eq!(
        unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) },
        0
    );
    let (sock, _peer) = unsafe {
        use std::os::fd::{FromRawFd, OwnedFd};
        (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1]))
    };
    assert_eq!(rejected_before_read(fs::File::from(sock)), None, "socket");
    assert_eq!(
        rejected_before_read(fs::File::open("/dev/null").unwrap()),
        None,
        "device (size 0, so only the type check can stop it)"
    );
}

#[cfg(unix)]
#[test]
fn a_regular_handle_within_the_cap_reaches_the_reader_and_a_big_one_does_not() {
    let t = TestDir::new("hooks-pointer-regular");
    let small = t.join("small.json");
    fs::write(&small, "{}").unwrap();
    let calls = std::cell::Cell::new(0);
    let got = scan::seen_in(fs::File::open(&small).unwrap(), |h| {
        calls.set(calls.get() + 1);
        scan::read_capped(h)
    });
    assert_eq!((got, calls.get()), (Some(scan::Seen::NoHooks), 1));
    let big = t.join("big.json");
    fs::write(&big, " ".repeat(scan::MAX_BYTES as usize + 1)).unwrap();
    assert_eq!(rejected_before_read(fs::File::open(&big).unwrap()), None);
}

#[cfg(unix)]
#[test]
fn a_file_swapped_for_a_link_after_the_check_is_not_followed() {
    let f = fx();
    settings(&f.repo, "settings.local.json", HOOKS);
    let file = f.repo.join(".claude").join("settings.local.json");
    let outside = f._t.join("elsewhere.json");
    fs::write(&outside, "{}").unwrap();
    // the swap lands between the "not a link" look and the open; followed it would read "no hooks" from outside
    let got = scan::look_with(
        &file,
        &f.repo,
        false,
        || {
            fs::remove_file(&file).unwrap();
            std::os::unix::fs::symlink(&outside, &file).unwrap();
        },
        scan::read_capped,
    );
    assert_eq!(got, None);
}

#[cfg(unix)]
#[test]
fn the_open_refuses_a_link_only_when_asked() {
    let f = fx();
    let real = f.repo.join("real.json");
    fs::write(&real, "{}").unwrap();
    let link = f.repo.join("link.json");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(scan::open_nonblocking(&link, true).is_err());
    assert!(scan::open_nonblocking(&link, false).is_ok());
    assert!(scan::open_nonblocking(&real, true).is_ok());
}

#[cfg(unix)]
#[test]
fn when_home_is_the_project_root_a_linked_user_claude_directory_is_still_read() {
    // dotfile managers link ~/.claude; HOME as the project root must not turn that into "unknown"
    let t = TestDir::new("hooks-pointer-home-root");
    let home = t.join("home");
    let dots = t.join("dotfiles").join("claude");
    fs::create_dir_all(home.join(".git")).unwrap();
    fs::create_dir_all(&dots).unwrap();
    std::os::unix::fs::symlink(&dots, home.join(".claude")).unwrap();
    // no hooks anywhere: the pointer shows, in a cwd below HOME and in HOME itself
    let cwd = home.join("work");
    fs::create_dir_all(&cwd).unwrap();
    assert!(no_hooks_pointer(&cwd, Some(&home), false).is_some());
    assert!(no_hooks_pointer(&home, Some(&home), false).is_some());
    // hooks in the linked directory silence it
    fs::write(dots.join("settings.json"), HOOKS).unwrap();
    assert_eq!(no_hooks_pointer(&cwd, Some(&home), false), None);
    // and a project-level .claude link below HOME is still not followed
    fs::remove_file(dots.join("settings.json")).unwrap();
    let other = TestDir::new("hooks-pointer-home-root-sub");
    let sub = cwd.join("proj");
    fs::create_dir_all(sub.join(".git")).unwrap();
    std::os::unix::fs::symlink(other.join(""), sub.join(".claude")).unwrap();
    assert_eq!(no_hooks_pointer(&sub, Some(&home), false), None);
}
