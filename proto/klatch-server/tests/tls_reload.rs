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
    let dir = Dir::new();
    let (server_ca, client_ca) = (Ca::new("server ca"), Ca::new(CANARY));
    let files = server_files(
        &dir,
        &server_ca.issue("klatch", Use::Server),
        &client_ca,
        0o600,
    );
    let mut args = tls_args(&files);
    let crl = with_crl.then(|| dir.0.join("pki/client.crl.pem"));
    if let Some(path) = &crl {
        write_file(path, &client_ca.crl_pem(&[], false), 0o644);
        args.extend(["--tls-client-crl".into(), path.display().to_string()]);
    }
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
fn a_flood_of_sighups_makes_a_bounded_number_of_reloads_and_the_server_still_works() {
    let r = rig(false);
    r.rotate_server();
    for _ in 0..100 {
        r.hup();
    }
    r.wait_for(OK, 1);
    std::thread::sleep(Duration::from_millis(2500));
    let reloads = r.count(OK);
    assert!(
        (1..=5).contains(&reloads),
        "{reloads} reloads for one flood:\n{}",
        r.dir.log()
    );
    assert_ne!(r.cert_now(), r.first_cert);
    assert!(r.server.child.id() > 0);
    // still serving, and a later signal still reloads
    let before = r.count(OK);
    r.rotate_server();
    r.reload_until(OK, before + 1);
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

#[test]
fn a_ca_file_that_blocks_cannot_hold_the_server_and_the_server_still_stops_on_sigterm() {
    let mut r = rig(false);
    // a FIFO nobody writes to: reading it never ends
    std::fs::remove_file(&r.files.ca).unwrap();
    let path = std::ffi::CString::new(r.files.ca.to_str().unwrap()).unwrap();
    // SAFETY: mkfifo gets a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let log = r.reload_until(FAILED, 1);
    assert!(log.contains("reload failed: timeout"), "{log}");
    assert_eq!(r.cert_now(), r.first_cert);
    // a second signal while the first build is stuck does not start another
    let log = r.reload_until(FAILED, 2);
    assert!(log.contains("an earlier reload has not finished"), "{log}");
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
