//! Reload on SIGHUP (#1492, refs #108, #1491): the server certificate and key, the client CA file and the client CRL file
//! are read again, with the checks of start, and swapped in whole or not at all. New connections use the new
//! configuration; connections already open keep theirs. A failed reload keeps the old configuration and logs one fixed
//! line naming the kind of failure (no file content, no subject, no serial). A flood of signals is coalesced. The plain
//! `http` server is not touched by SIGHUP. Certificates are made at test time (rcgen), dates relative to now; every
//! wait has a deadline, so a regression fails and never hangs.

mod tls_common;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tls_common::wire::*;
use tls_common::*;

/// A serial no port number can contain, and a subject no log line may hold.
const REVOKED: u64 = 8_675_309;
const GOOD: u64 = 8_675_311;
const CANARY: &str = "canary-subject-7731";

struct Rig {
    dir: Dir,
    server_ca: Ca,
    client_ca: Ca,
    files: Files,
    crl: Option<PathBuf>,
    server: Server,
    first_cert: Vec<u8>,
}

fn rig(with_crl: bool) -> Rig {
    rig_with(with_crl.then_some(30 * 86_400))
}

/// A server whose client CRL (if any) is due in `crl_due` seconds from now (negative: already past).
fn rig_with(crl_due: Option<i64>) -> Rig {
    rig_full(crl_due, &[])
}

/// [`rig_with`] and more options for the server.
fn rig_full(crl_due: Option<i64>, extra: &[&str]) -> Rig {
    let dir = Dir::new();
    let (server_ca, client_ca) = (Ca::new("server ca"), Ca::new(CANARY));
    let files = server_files(
        &dir,
        &server_ca.issue("klatch", Use::Server),
        &client_ca,
        0o600,
    );
    let mut args = tls_args(&files);
    let crl = crl_due.map(|_| dir.0.join("pki/client.crl.pem"));
    if let (Some(path), Some(due)) = (&crl, crl_due) {
        write_file(path, &client_ca.crl_pem_due(due), 0o644);
        args.extend(["--tls-client-crl".into(), path.display().to_string()]);
    }
    args.extend(extra.iter().map(|s| s.to_string()));
    let server = Server::started(&dir, "https", &args);
    let mut r = Rig {
        dir,
        server_ca,
        client_ca,
        files,
        crl,
        server,
        first_cert: Vec::new(),
    };
    let cfg = r.client_config(GOOD);
    r.first_cert = served_cert(r.server.addr, &cfg);
    r
}

impl Rig {
    fn client_config(&self, serial: u64) -> Arc<rustls::ClientConfig> {
        let leaf = self
            .client_ca
            .issue_serial("worker", Use::Client, Some(serial));
        client_config(&self.server_ca, Some(&leaf), None)
    }

    /// Put another server certificate and its key in the files (not signalled).
    fn rotate_server(&self) {
        let leaf = self.server_ca.issue("klatch", Use::Server);
        rewrite_file(&self.files.cert, &leaf.cert_pem, 0o644);
        rewrite_file(&self.files.key, &leaf.key_pem, 0o600);
    }

    fn hup(&self) {
        // SAFETY: kill has no preconditions; the pid is this test's own child.
        unsafe {
            libc::kill(self.server.child.id() as libc::pid_t, libc::SIGHUP);
        }
    }

    fn count(&self, needle: &str) -> usize {
        self.dir.log().matches(needle).count()
    }

    /// Signal and wait (with a deadline) until the log holds `n` lines of `needle`; the whole log on success.
    fn reload_until(&self, needle: &str, n: usize) -> String {
        self.hup();
        self.wait_for(needle, n)
    }

    fn wait_for(&self, needle: &str, n: usize) -> String {
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            let log = self.dir.log();
            if log.matches(needle).count() >= n {
                return log;
            }
            assert!(
                Instant::now() < until,
                "no {needle:?} x{n} in the log:\n{log}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn cert_now(&self) -> Vec<u8> {
        served_cert(self.server.addr, &self.client_config(GOOD))
    }
}

/// The server certificate (DER) a client is shown.
fn served_cert(addr: SocketAddr, config: &Arc<rustls::ClientConfig>) -> Vec<u8> {
    let tls = tls_connect(addr, config).expect("handshake");
    let certs = tls.conn.peer_certificates().expect("server certificate");
    certs[0].as_ref().to_vec()
}

const OK: &str = "https: reload ok";
const FAILED: &str = "https: reload failed: ";

/// A log that must tell nothing about files, subjects or serials.
fn assert_quiet(log: &str) {
    for secret in ["PRIVATE KEY", CANARY, "worker", &REVOKED.to_string()] {
        assert!(!log.contains(secret), "the log holds {secret:?}:\n{log}");
    }
}

// ---------------------------------------------------------------------------------------------------------------

#[test]
fn a_new_server_certificate_is_served_to_new_connections_after_sighup_and_not_before() {
    let r = rig(false);
    r.rotate_server();
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(r.cert_now(), r.first_cert, "files are not watched");
    r.reload_until(OK, 1);
    let now = r.cert_now();
    assert_ne!(now, r.first_cert, "the old certificate is still served");
    assert!(!r.dir.log().contains(FAILED), "{}", r.dir.log());
}

#[test]
fn an_open_stream_survives_a_reload_and_keeps_working() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let r = rig(false);
    let token = issue_credential(&r.dir, "alice");
    let net = Net {
        addr: r.server.addr,
        tls: Some(r.client_config(GOOD)),
    };
    let mut stream = Sse::open(&net, "alice", &token);
    stream.recv().expect("first frame");
    r.rotate_server();
    r.reload_until(OK, 1);
    let joined = stream.request("join", serde_json::json!({"room": "conf"}));
    assert_eq!(joined["ok"], true, "{joined}");
    assert_ne!(r.cert_now(), r.first_cert);
}

#[test]
fn a_reload_that_fails_keeps_the_old_configuration_logs_the_kind_and_a_later_good_reload_works() {
    let r = rig(true);
    let crl = r.crl.clone().unwrap();
    let good_crl = r.client_ca.crl_pem(&[], false);
    let good_ca = r.client_ca.pem.clone();
    let good_cert = std::fs::read_to_string(&r.files.cert).unwrap();
    let good_key = std::fs::read_to_string(&r.files.key).unwrap();
    let other = r.server_ca.issue("klatch", Use::Server);
    let mut failures = 0;
    let mut check = |what: &str, kind: &str| {
        failures += 1;
        let log = r.reload_until(FAILED, failures);
        assert!(
            log.contains(&format!("{FAILED}{kind}")),
            "{what}: wanted kind {kind:?}:\n{log}"
        );
        assert_eq!(r.cert_now(), r.first_cert, "{what}: the config changed");
        assert_eq!(r.count(OK), 0, "{what}: a reload succeeded:\n{log}");
        assert_quiet(&log);
    };
    // a CA file with no certificate
    rewrite_file(&r.files.ca, "no certificate in here\n", 0o644);
    check("empty CA file", "client CA file");
    rewrite_file(&r.files.ca, &good_ca, 0o644);
    // a CRL file that is not a CRL
    rewrite_file(&crl, "not a crl\n", 0o644);
    check("garbage CRL", "client CRL file");
    // a CRL file anyone may write
    rewrite_file(&crl, &good_crl, 0o666);
    check("writable CRL", "client CRL file");
    // two CRLs of one issuer
    let twice = format!(
        "{}{}",
        r.client_ca.crl_pem(&[REVOKED], false),
        r.client_ca.crl_pem(&[], false)
    );
    rewrite_file(&crl, &twice, 0o644);
    check("two CRLs of one issuer", "client CRL file");
    rewrite_file(&crl, &good_crl, 0o644);
    // a CRL that has no nextUpdate rule problem is the CRL checks of start: an unreadable CRL file
    std::fs::remove_file(&crl).unwrap();
    check("missing CRL", "client CRL file");
    rewrite_file(&crl, &good_crl, 0o644);
    // a key that does not belong to the certificate
    rewrite_file(&r.files.key, &other.key_pem, 0o600);
    check("key does not fit", "key and certificate do not fit");
    // a key anyone may read
    rewrite_file(&r.files.key, &good_key, 0o644);
    check("key mode", "server key file");
    rewrite_file(&r.files.key, &good_key, 0o600);
    // a certificate file that holds none
    rewrite_file(&r.files.cert, "nothing\n", 0o644);
    check("no certificate", "server certificate");
    rewrite_file(&r.files.cert, &good_cert, 0o644);
    // everything is restored: the next reload succeeds, with the old certificate (same files as at start)
    r.reload_until(OK, 1);
    assert_eq!(r.cert_now(), r.first_cert);
    assert_quiet(&r.dir.log());
}

#[test]
fn a_client_accepted_before_is_refused_after_a_reload_with_a_crl_that_lists_it() {
    let r = rig(true);
    let revoked = r.client_config(REVOKED);
    let good = r.client_config(GOOD);
    assert_eq!(get_status(r.server.addr, &revoked, "/nope").unwrap(), 404);
    rewrite_file(
        r.crl.as_ref().unwrap(),
        &r.client_ca.crl_pem(&[REVOKED], false),
        0o644,
    );
    r.reload_until(OK, 1);
    let err = get_status(r.server.addr, &revoked, "/nope")
        .expect_err("a certificate the new CRL lists got an answer");
    assert!(err.to_string().contains("CertificateRevoked"), "{err}");
    assert_eq!(get_status(r.server.addr, &good, "/nope").unwrap(), 404);
    assert_quiet(&r.dir.log());
}

#[test]
fn the_warnings_of_the_crl_checks_print_again_for_the_files_a_reload_reads() {
    let r = rig(true);
    assert_eq!(r.count("warning"), 0, "{}", r.dir.log());
    rewrite_file(
        r.crl.as_ref().unwrap(),
        &r.client_ca.crl_pem_due(3 * 86_400),
        0o644,
    );
    let log = r.reload_until(OK, 1);
    assert!(
        log.contains("warning: CRL 1 of 1 passes its nextUpdate"),
        "{log}"
    );
    assert_quiet(&log);
}

#[test]
fn one_sighup_is_exactly_one_reload() {
    let r = rig(false);
    r.rotate_server();
    r.reload_until(OK, 1);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(r.count(OK), 1, "{}", r.dir.log());
}

#[test]
fn a_flood_of_sighups_makes_a_bounded_number_of_reloads_stops_and_the_server_still_works() {
    let r = rig(false);
    r.rotate_server();
    for _ in 0..100 {
        r.hup();
    }
    r.wait_for(OK, 1);
    // one reload may be pending behind the first (the spacing is half a second): wait it out, then nothing more may come
    std::thread::sleep(Duration::from_millis(1500));
    let settled = r.count(OK);
    assert!(
        (1..=3).contains(&settled),
        "{settled} reloads for one flood:\n{}",
        r.dir.log()
    );
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        r.count(OK),
        settled,
        "reloads went on after the flood:\n{}",
        r.dir.log()
    );
    assert_ne!(r.cert_now(), r.first_cert);
    // a later signal still reloads
    r.rotate_server();
    r.reload_until(OK, settled + 1);
}

#[test]
fn a_sustained_flood_is_spaced_not_one_reload_per_signal() {
    let r = rig(false);
    // 100 signals over two seconds: with a start at most every half second that is a handful of reloads, never a hundred
    for _ in 0..100 {
        r.hup();
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(1000));
    let n = r.count(OK);
    assert!((1..=7).contains(&n), "{n} reloads:\n{}", r.dir.log());
    assert_eq!(r.cert_now(), r.first_cert);
}

#[test]
fn sighup_does_nothing_to_the_plain_http_server() {
    let dir = Dir::new();
    let mut server = Server::started(&dir, "http", &[]);
    // SAFETY: kill has no preconditions; the pid is this test's own child.
    unsafe {
        libc::kill(server.child.id() as libc::pid_t, libc::SIGHUP);
    }
    std::thread::sleep(Duration::from_millis(700));
    assert!(
        server.child.try_wait().unwrap().is_none(),
        "the server stopped on SIGHUP:\n{}",
        dir.log()
    );
    let net = Net {
        addr: server.addr,
        tls: None,
    };
    assert_eq!(status_of(&net, ("GET", "/v1/stream"), &[], b""), 401);
    assert!(!dir.log().contains("reload"), "{}", dir.log());
}

/// A FIFO at `path` (a named pipe nobody writes to: opening or reading it without care never returns).
fn mkfifo(path: &std::path::Path) {
    let c = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
    // SAFETY: mkfifo gets a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
}

fn stops_on_sigterm(r: &mut Rig) {
    // SAFETY: kill has no preconditions; the pid is this test's own child.
    unsafe {
        libc::kill(r.server.child.id() as libc::pid_t, libc::SIGTERM);
    }
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        if r.server.child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(Instant::now() < until, "the server did not stop");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_fifo_as_the_ca_file_is_refused_at_once_and_a_good_file_renamed_over_it_reloads() {
    let mut r = rig(false);
    let good = std::fs::read_to_string(&r.files.ca).unwrap();
    std::fs::remove_file(&r.files.ca).unwrap();
    mkfifo(&r.files.ca);
    let log = r.reload_until(FAILED, 1);
    assert!(log.contains("reload failed: client CA file"), "{log}");
    assert_eq!(r.cert_now(), r.first_cert);
    // the fix an operator makes: a good file renamed over the pipe; the next signal is not "an earlier reload"
    let new = r.files.ca.with_extension("new");
    write_file(&new, &good, 0o644);
    std::fs::rename(&new, &r.files.ca).unwrap();
    let log = r.reload_until(OK, 1);
    assert!(!log.contains("has not finished"), "{log}");
    stops_on_sigterm(&mut r);
}

/// Files for a server that is not started, in a directory of its own.
fn unstarted() -> (Dir, Files) {
    let dir = Dir::new();
    let (server_ca, client_ca) = (Ca::new("server ca"), Ca::new("client ca"));
    let files = server_files(
        &dir,
        &server_ca.issue("klatch", Use::Server),
        &client_ca,
        0o600,
    );
    (dir, files)
}

fn start_refused_for(dir: &Dir, files: &Files, needle: &str) {
    let mut args = vec!["--listen".to_string(), "127.0.0.1:0".to_string()];
    args.extend(tls_args(files));
    let (code, err) = refused_start(dir, "https", &args);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains(needle), "wanted {needle:?} in: {err}");
    assert!(!err.contains("listening on"), "{err}");
    let _ = std::fs::remove_file(dir.0.join("err.log"));
}

#[test]
fn the_ca_and_certificate_files_are_checked_like_the_key_at_start() {
    let (dir, files) = unstarted();
    let (ca, cert) = (files.ca.clone(), files.cert.clone());
    let (good_ca, good_cert) = (
        std::fs::read_to_string(&ca).unwrap(),
        std::fs::read_to_string(&cert).unwrap(),
    );
    for path in [&ca, &cert] {
        let what = if path == &ca {
            "client CA file"
        } else {
            "server certificate"
        };
        let good = if path == &ca { &good_ca } else { &good_cert };
        // a named pipe: refused, not waited for
        std::fs::remove_file(path).unwrap();
        mkfifo(path);
        start_refused_for(&dir, &files, &format!("the {what}"));
        start_refused_for(&dir, &files, "not a regular file");
        // a link, even to a good file
        std::fs::remove_file(path).unwrap();
        let target = dir.0.join("elsewhere.pem");
        write_file(&target, good, 0o644);
        std::os::unix::fs::symlink(&target, path).unwrap();
        start_refused_for(&dir, &files, "symbolic link");
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(&target).unwrap();
        // larger than 4 MiB
        write_file(path, good, 0o644);
        let f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        f.set_len(5 * 1024 * 1024).unwrap();
        start_refused_for(&dir, &files, "larger than 4 MiB");
        rewrite_file(path, good, 0o644);
    }
    // the CA file is a trust anchor: nobody else may write it
    for mode in [0o666, 0o664, 0o646] {
        rewrite_file(&ca, &good_ca, mode);
        start_refused_for(&dir, &files, "writable by group or other");
    }
    rewrite_file(&ca, &good_ca, 0o444);
    // and with everything in order it starts (the checks refuse nothing else)
    let args = tls_args(&files);
    let _ = Server::started(&dir, "https", &args);
}

#[test]
fn a_reload_refuses_a_ca_file_that_is_a_link_or_writable_by_others_and_keeps_serving() {
    let r = rig(false);
    let good = std::fs::read_to_string(&r.files.ca).unwrap();
    let attacker = Ca::new("attacker ca");
    let elsewhere = r.dir.0.join("attacker.pem");
    write_file(&elsewhere, &attacker.pem, 0o644);
    std::fs::remove_file(&r.files.ca).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &r.files.ca).unwrap();
    let log = r.reload_until(FAILED, 1);
    assert!(log.contains("reload failed: client CA file"), "{log}");
    // the attacker's client certificate is not let through
    let bad = attacker.issue("worker", Use::Client);
    let bad = client_config(&r.server_ca, Some(&bad), None);
    assert!(get_status(r.server.addr, &bad, "/nope").is_err());
    std::fs::remove_file(&r.files.ca).unwrap();
    write_file(&r.files.ca, &good, 0o666);
    let log = r.reload_until(FAILED, 2);
    assert!(log.contains("reload failed: client CA file"), "{log}");
    assert_eq!(r.count(OK), 0, "{log}");
    assert_eq!(r.cert_now(), r.first_cert);
    rewrite_file(&r.files.ca, &good, 0o644);
    r.reload_until(OK, 1);
}

#[test]
fn a_reload_refuses_a_crl_that_has_expired_while_the_one_in_force_is_in_date() {
    let r = rig(true);
    let crl = r.crl.clone().unwrap();
    let new = crl.with_extension("new");
    write_file(&new, &r.client_ca.crl_pem_due(-86_400), 0o644);
    std::fs::rename(&new, &crl).unwrap();
    let log = r.reload_until(FAILED, 1);
    assert!(log.contains("reload failed: client CRL file"), "{log}");
    assert!(log.contains("ALERT"), "the expiry is an alert line:\n{log}");
    // #1641: the file never took effect, so no line may say that its clients are refused
    assert!(
        !log.contains("every client certificate of its issuer is refused (expiry is enforced)"),
        "a refused file is not in force:\n{log}"
    );
    assert!(
        log.contains("not taken"),
        "the alert says it was not taken:\n{log}"
    );
    assert_eq!(r.count(OK), 0, "{log}");
    // the old CRL still serves: a client is not locked out
    let good = r.client_config(GOOD);
    assert_eq!(get_status(r.server.addr, &good, "/nope").unwrap(), 404);
    assert_quiet(&log);
    // a current CRL then reloads
    let new = crl.with_extension("new");
    write_file(&new, &r.client_ca.crl_pem_due(20 * 86_400), 0o644);
    std::fs::rename(&new, &crl).unwrap();
    r.reload_until(OK, 1);
}

#[test]
fn when_the_crl_in_force_has_expired_too_a_reload_takes_the_new_file_and_says_so_as_an_alert() {
    let r = rig_with(Some(-86_400));
    let crl = r.crl.clone().unwrap();
    let new = crl.with_extension("new");
    write_file(&new, &r.client_ca.crl_pem_due(-3_600), 0o644);
    std::fs::rename(&new, &crl).unwrap();
    let log = r.reload_until(OK, 1);
    assert!(log.contains("ALERT"), "{log}");
}

/// Replace the CRL file by one due in `due` seconds (negative: past), by rename, as an operator does.
fn put_crl(r: &Rig, due: i64) {
    let crl = r.crl.clone().unwrap();
    let new = crl.with_extension("new");
    write_file(&new, &r.client_ca.crl_pem_due(due), 0o644);
    std::fs::rename(&new, &crl).unwrap();
}

/// #1641: the due time of the CRL a reload took is the one the next reload compares against.
#[test]
fn a_fresh_crl_taken_at_a_reload_is_the_one_in_force_for_the_next_reload() {
    let r = rig_with(Some(-86_400));
    put_crl(&r, 20 * 86_400);
    r.reload_until(OK, 1);
    put_crl(&r, -3_600);
    let log = r.reload_until(FAILED, 1);
    assert!(log.contains("reload failed: client CRL file"), "{log}");
    assert_eq!(r.count(OK), 1, "the expired file was taken:\n{log}");
}

// The lapse of a CRL taken at a reload (a CRL due in 5 s, then past it) is tested with an injected clock in
// `src/tls_crl_tests.rs` (#1704): a test that sleeps past a due time can flake on a slow runner.

#[test]
fn a_reload_still_applies_to_new_connections_while_a_silent_socket_holds_a_handshake_slot() {
    // a pool of two handshakes, one held by a socket that never says hello (#1493): the new connection takes the other
    let r = rig_full(None, &["--max-handshakes", "2"]);
    let silent = std::net::TcpStream::connect(r.server.addr).unwrap();
    r.rotate_server();
    r.reload_until(OK, 1);
    let now = r.cert_now();
    assert_ne!(now, r.first_cert, "the old certificate is still served");
    // the silent socket is still waiting in its handshake: it was not cut by the reload
    use std::io::Read;
    silent.set_nonblocking(true).unwrap();
    let mut byte = [0u8; 1];
    let still_open = matches!(
        (&silent).read(&mut byte),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
    );
    assert!(still_open, "the silent socket was closed");
}
