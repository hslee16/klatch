//! `--tls-client-intermediates` at reload (#1623, refs #1492): SIGHUP reads the intermediates file again with the checks
//! of start, together with the CRL file, and swaps the whole configuration in or none of it. A CRL whose issuer matches
//! neither a CA nor a listed intermediate is refused at reload (the old configuration stays); a changed intermediates
//! file is picked up; a bad one is refused and the old configuration stays. Every wait has a deadline.

mod tls_common;

use rcgen::KeyUsagePurpose::{CrlSign, KeyCertSign};
use std::time::{Duration, Instant};
use tls_common::*;

const A_BAD: u64 = 1111;
const B_BAD: u64 = 2222;

struct Rig {
    dir: Dir,
    server_ca: Ca,
    a: Ca,
    b: Ca,
    files: Files,
    server: Server,
}

fn rig() -> Rig {
    let dir = Dir::new();
    let (server_ca, root) = (Ca::new("server ca"), Ca::new("client ca"));
    let files = server_files(&dir, &server_ca.issue("klatch", Use::Server), &root, 0o600);
    let a = root.intermediate("inter a", vec![KeyCertSign, CrlSign]);
    let b = root.intermediate("inter b", vec![KeyCertSign, CrlSign]);
    write_file(&dir.0.join("pki/c.crl"), &a.crl_pem(&[A_BAD], false), 0o644);
    write_file(&dir.0.join("pki/i.pem"), &a.pem, 0o644);
    let mut args = tls_args(&files);
    args.extend([
        "--tls-client-crl".into(),
        dir.0.join("pki/c.crl").display().to_string(),
        "--tls-client-intermediates".into(),
        dir.0.join("pki/i.pem").display().to_string(),
    ]);
    let server = Server::started(&dir, "https", &args);
    Rig {
        dir,
        server_ca,
        a,
        b,
        files,
        server,
    }
}

impl Rig {
    fn crl(&self, text: &str) {
        rewrite_file(&self.dir.0.join("pki/c.crl"), text, 0o644);
    }

    fn inters(&self, text: &str) {
        rewrite_file(&self.dir.0.join("pki/i.pem"), text, 0o644);
    }

    /// Does a client of `ca` with this serial get in?
    fn admitted(&self, ca: &Ca, serial: u64) -> bool {
        let leaf = ca.issue_serial("worker", Use::Client, Some(serial));
        let config = client_config(&self.server_ca, Some(&leaf), None);
        match get_status(self.server.addr, &config, "/nope") {
            Ok(status) => {
                assert_eq!(status, 404);
                true
            }
            Err(e) => {
                assert!(e.to_string().contains("CertificateRevoked"), "{e}");
                false
            }
        }
    }

    /// Signal, and wait (deadline) until the log holds `n` of `needle`; the log.
    fn reload_until(&self, needle: &str, n: usize) -> String {
        // SAFETY: kill has no preconditions; the pid is this test's own child.
        unsafe {
            libc::kill(self.server.child.id() as libc::pid_t, libc::SIGHUP);
        }
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
}

#[test]
fn a_reload_picks_up_a_changed_intermediates_file_and_refuses_a_crl_that_matches_none() {
    let r = rig();
    assert!(!r.admitted(&r.a, A_BAD) && r.admitted(&r.b, B_BAD));
    assert!(!r.dir.log().contains("warning:"), "{}", r.dir.log());

    // (1) the CRL of B, which no CA and no listed intermediate (only A) matches: refused, the old configuration stays
    r.crl(&r.b.crl_pem(&[B_BAD], false));
    let log = r.reload_until("https: reload failed: client CRL file;", 1);
    assert!(
        log.contains("keeping the previous TLS configuration"),
        "{log}"
    );
    assert!(!r.admitted(&r.a, A_BAD), "the old CRL is still in force");
    assert!(r.admitted(&r.b, B_BAD), "the refused CRL is not in force");

    // (2) the intermediates file now lists B (and not A): the same CRL is accepted, and B's client is revoked
    r.inters(&r.b.pem);
    let log = r.reload_until("https: reload ok", 1);
    assert!(
        log.contains("the client CRL file and the client intermediates file"),
        "{log}"
    );
    assert!(!r.admitted(&r.b, B_BAD), "the new CRL is in force");
    assert!(r.admitted(&r.a, A_BAD), "A's CRL is no longer in the file");

    // (3) a bad intermediates file is refused, and the configuration of (2) stays
    for (n, text) in ["junk\n", ""].into_iter().enumerate() {
        r.inters(text);
        r.reload_until("https: reload failed: client intermediates file;", n + 1);
        assert!(!r.admitted(&r.b, B_BAD), "the old configuration stays");
    }
    // a symbolic link and a group-writable file too
    let link = r.dir.0.join("pki/link.pem");
    std::os::unix::fs::symlink(r.dir.0.join("pki/c.crl"), &link).unwrap();
    std::fs::remove_file(r.dir.0.join("pki/i.pem")).unwrap();
    std::os::unix::fs::symlink(&link, r.dir.0.join("pki/i.pem")).unwrap();
    r.reload_until("https: reload failed: client intermediates file;", 3);
    std::fs::remove_file(r.dir.0.join("pki/i.pem")).unwrap();
    write_file(&r.dir.0.join("pki/i.pem"), &r.b.pem, 0o664);
    r.reload_until("https: reload failed: client intermediates file;", 4);
    assert!(!r.admitted(&r.b, B_BAD), "the old configuration stays");

    // the log of all of it names no subject, no serial and no content
    let log = r.dir.log();
    if let Some(secret) = leaked(&log) {
        panic!("{secret:?} in:\n{log}");
    }
    let _ = &r.files;
}

/// A secret the log of a reload names, if any. The line of the listening address holds a random port, which can contain
/// the digits of a serial by chance (#1791), so that line is not searched.
fn leaked(log: &str) -> Option<&'static str> {
    let text: String = log
        .lines()
        .filter(|l| !l.contains(" listening on "))
        .collect::<Vec<_>>()
        .join("\n");
    ["inter a", "inter b", "1111", "2222", "BEGIN"]
        .into_iter()
        .find(|s| text.contains(s))
}

#[test]
fn the_random_port_of_the_listening_line_is_not_a_leaked_serial() {
    // #1791: a port such as 51111 or 62222 is not a serial in the log
    for port in [51111, 62222] {
        let log = format!("klatch-server: https listening on 127.0.0.1:{port}\n");
        assert_eq!(leaked(&log), None, "{log}");
    }
    // while a serial or a subject in any other line is still found
    assert_eq!(
        leaked("x listening on 127.0.0.1:1\nreload failed 1111\n"),
        Some("1111")
    );
    assert_eq!(leaked("a: inter b\n"), Some("inter b"));
}
