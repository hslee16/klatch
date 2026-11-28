//! The daemon tests (#1633, split from `daemon.rs`).

use super::peer::{PeerCred, WrongUid};
use super::*;

/// #1640: with a cap of one, a connection whose handler panics must give its slot back, the accept loop must go on,
/// and the loop must end normally at stop. Every connect and read has a deadline.
#[test]
fn a_daemon_handler_that_panics_gives_its_slot_back_and_the_loop_goes_on() {
    let dir = TestDir::new("panic");
    let path = dir.0.join("d.sock");
    let listener = UnixListener::bind(&path).unwrap();
    listener.set_nonblocking(true).unwrap();
    let stop = AtomicBool::new(false);
    let first = AtomicBool::new(true);
    let mut said = String::new();
    std::thread::scope(|scope| {
        let loop_ended = scope.spawn(|| {
            accept_loop(&listener, &stop, (own_uid(), 1), &|mut s, _| {
                if first.swap(false, Ordering::SeqCst) {
                    panic!("a handler fault");
                }
                let _ = s.write_all(b"ok");
            });
        });
        let ask = || {
            let mut c = UnixStream::connect(&path).unwrap();
            c.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut out = Vec::new();
            let _ = c.read_to_end(&mut out);
            String::from_utf8_lossy(&out).into_owned()
        };
        let _ = ask();
        let until = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < until {
            said = ask();
            if said.ends_with("ok") {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        stop.store(true, Ordering::SeqCst);
        assert!(loop_ended.join().is_ok(), "the panic ended the accept loop");
    });
    assert!(
        said.ends_with("ok"),
        "the slot of the panicked handler was kept: {said}"
    );
}

/// #1640: on stop every open connection is closed, so a handler blocked on its socket returns and the loop ends.
/// The handler's own read gives up after 4 s, so a regression fails the assertion instead of hanging.
#[test]
fn open_connections_are_closed_when_the_accept_loop_stops() {
    let dir = TestDir::new("stop");
    let path = dir.0.join("d.sock");
    let listener = UnixListener::bind(&path).unwrap();
    listener.set_nonblocking(true).unwrap();
    let (stop, serving) = (AtomicBool::new(false), AtomicBool::new(false));
    let mut closed = false;
    std::thread::scope(|scope| {
        let loop_ended = scope.spawn(|| {
            accept_loop(&listener, &stop, (own_uid(), 4), &|mut s, _| {
                s.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
                serving.store(true, Ordering::SeqCst);
                let _ = s.read(&mut [0u8; 1]);
            });
        });
        let mut c = UnixStream::connect(&path).unwrap();
        let until = std::time::Instant::now() + Duration::from_secs(5);
        while !serving.load(Ordering::SeqCst) && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        stop.store(true, Ordering::SeqCst);
        c.set_read_timeout(Some(Duration::from_millis(1500)))
            .unwrap();
        closed = matches!(c.read(&mut [0u8; 1]), Ok(0));
        assert!(loop_ended.join().is_ok());
    });
    assert!(closed, "an open connection was left open at stop");
}

/// A private (0700) directory that goes when the test ends, even on a failed assertion. The tests here do not use
/// `bind`, which sets the process-wide umask for a moment and would race the tests that create directories.
struct TestDir(PathBuf);

impl TestDir {
    fn new(tag: &str) -> TestDir {
        use std::os::unix::fs::DirBuilderExt;
        let dir = PathBuf::from(format!("/tmp/kp{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        TestDir(dir)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Whether `join gated` is admitted on the backend `unidentified_backend` gives a peer, with `probe` standing in for
/// the process-tree walk. The room requires a capability only a verified harness has.
fn joins_gated(probe: fn(u32) -> bool, evidence: &PeerEvidence) -> bool {
    use klatch::gateway::{AllowAll, Gateway, SystemClock};
    let dir = PathBuf::from(format!(
        "/tmp/kh{}-{:?}",
        std::process::id(),
        probe as usize
    ));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("rooms.db");
    let gw = SharedGateway::spawn(move || {
        let store = klatch::store::SqliteStore::open(&path, &klatch::store::insecure_test_key())?;
        let g = Gateway::new(store, SystemClock, AllowAll);
        g.admin_set_room_require(
            &Principal::unattributed("admin"),
            "gated",
            Some("refuse:session-identity"),
        )?;
        Ok(g)
    })
    .unwrap();
    let shared = Shared {
        backend: gw.session_backend(),
        claude_code_at: probe,
        first_frame: Duration::from_secs(1),
        config: SessionConfig::default(),
    };
    let backend = unidentified_backend(&shared, evidence);
    let who = Principal::unattributed("unmapped-process");
    let script = "{\"v\":1,\"type\":\"request\",\"id\":\"j\",\"method\":\"join\",\"params\":{\"room\":\"gated\"}}\n";
    let mut out = Vec::new();
    Session::over(&backend, &who, &shared.config)
        .without_hello()
        .serve(Cursor::new(script.as_bytes().to_vec()), &mut out)
        .unwrap();
    drop((backend, shared, gw));
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8(out).unwrap().contains("\"ok\":true")
}

/// #1096: only a peer whose pid the walk verifies as Claude Code gets the harness; every other peer is the floor.
#[test]
fn a_peer_in_a_verified_claude_code_session_joins_as_claude_code_and_any_other_as_the_floor() {
    let peer = PeerEvidence::Process { pid: 77, uid: 501 };
    assert!(joins_gated(|pid| pid == 77, &peer));
    assert!(!joins_gated(|_| false, &peer));
    // the probe is asked about the kernel's pid and about nothing else
    assert!(!joins_gated(
        |pid| pid == 77,
        &PeerEvidence::Process { pid: 78, uid: 501 }
    ));
    assert!(!joins_gated(
        |_| true,
        &PeerEvidence::DeclaredName("claude".into())
    ));
}

/// A fake credential, as the kernel would report it: this is the whole of the uid check, and a connection from
/// another uid cannot be made in a test.
#[test]
fn only_the_daemons_own_uid_is_accepted_and_the_pid_is_evidence_not_a_name() {
    let own = 501;
    let ev = local_evidence(
        PeerCred {
            pid: 4242,
            uid: 501,
        },
        own,
    )
    .unwrap();
    assert_eq!(
        ev,
        PeerEvidence::Process {
            pid: 4242,
            uid: 501
        }
    );
    for other in [0, 1, 500, 502, u32::MAX] {
        assert_eq!(
            local_evidence(
                PeerCred {
                    pid: 4242,
                    uid: other
                },
                own
            ),
            Err(WrongUid),
            "uid {other}"
        );
    }
    // root is another user too, when the daemon is not root
    assert_eq!(
        local_evidence(PeerCred { pid: 1, uid: 0 }, own),
        Err(WrongUid)
    );
    // whatever the pid, the principal is the same unmapped name: the pid never becomes a name
    for pid in [1, 4242, 0] {
        let ev = local_evidence(PeerCred { pid, uid: own }, own).unwrap();
        let p = unidentified(&ev);
        assert_eq!(p.member, "unmapped-process");
        assert_eq!(p.attribution, klatch::domain::Attribution::Unattributed);
    }
}

#[test]
fn a_directory_must_be_the_daemons_and_closed_to_group_and_others() {
    let d = Path::new("/tmp/x");
    assert!(directory_problem(d, 501, 0o40700, 501).is_none());
    assert!(directory_problem(d, 501, 0o40750, 501).is_none());
    assert!(directory_problem(d, 501, 0o40755, 501).is_none());
    for mode in [0o40777, 0o40770, 0o40707, 0o41777, 0o40722] {
        let p = directory_problem(d, 501, mode, 501).unwrap();
        assert!(
            p.contains("not safe") && p.contains("writable"),
            "{mode:o}: {p}"
        );
    }
    let p = directory_problem(d, 0, 0o40700, 501).unwrap();
    assert!(
        p.contains("not safe") && p.contains("owned by uid 0"),
        "{p}"
    );
}

#[test]
fn a_socket_path_longer_than_the_limit_is_refused_and_the_limit_is_named() {
    let limit = path_limit();
    assert!((90..=108).contains(&limit), "{limit}");
    let exact = "a".repeat(limit);
    assert!(path_problem(Path::new(&exact), limit).is_none());
    let long = "a".repeat(limit + 1);
    let p = path_problem(Path::new(&long), limit).unwrap();
    assert!(
        p.contains("too long") && p.contains(&format!("{limit} bytes")),
        "{p}"
    );
}

#[test]
fn the_first_line_is_capped_and_a_closed_peer_has_none() {
    let mut r = Cursor::new(b"abc\nrest".to_vec());
    assert_eq!(first_line(&mut r, 10).unwrap().unwrap(), b"abc\n");
    let mut r = Cursor::new(Vec::new());
    assert!(first_line(&mut r, 10).unwrap().is_none());
    let mut r = Cursor::new(vec![b'x'; 100]);
    assert_eq!(
        first_line(&mut r, 10).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    // exactly the cap, with its newline, is fine
    let mut line = vec![b'x'; 9];
    line.push(b'\n');
    let mut r = Cursor::new(line);
    assert!(first_line(&mut r, 10).unwrap().is_some());
}

#[test]
fn identify_is_recognised_only_as_a_well_formed_request() {
    assert!(matches!(
        classify(br#"{"v":1,"type":"request","id":"a","method":"identify","params":{"name":"x"}}"#),
        First::Identify { .. }
    ));
    for other in [
        &br#"{"v":1,"type":"request","id":"a","method":"post","params":{}}"#[..],
        br#"{"v":1,"type":"request","method":"identify"}"#,
        br#"{"v":1,"type":"ack","room":"r","seq":1}"#,
        b"garbage",
    ] {
        assert!(matches!(classify(other), First::Other));
    }
}

#[test]
fn options_need_a_socket_and_take_two_limits() {
    assert!(parse(&[]).is_err());
    assert!(parse(&["--socket"]).is_err());
    assert!(parse(&["--socket", "/tmp/a", "--bogus"]).is_err());
    assert!(parse(&["--socket", "/tmp/a", "--max-connections", "0"]).is_err());
    let o = parse(&[
        "--socket",
        "/tmp/a",
        "--max-connections",
        "3",
        "--first-frame-secs",
        "2",
    ])
    .unwrap();
    assert_eq!(o.max_connections, 3);
    assert_eq!(o.first_frame, Duration::from_secs(2));
}

/// #1776: an unknown option the operator typed is echoed in the refusal; a terminal escape, a bidi override or a
/// zero-width character in it must not reach the terminal raw.
#[test]
fn an_unknown_option_is_echoed_escaped() {
    let e = format!(
        "{:#}",
        parse(&["--bad\u{1b}[2J\u{202e}\u{200b}"]).expect_err("an unknown option is refused")
    );
    assert!(e.contains("\\u{1b}"), "{e}");
    assert!(
        !e.chars().any(|c| c.is_control()) && !e.contains(['\u{202e}', '\u{200b}']),
        "{e:?}"
    );
}
