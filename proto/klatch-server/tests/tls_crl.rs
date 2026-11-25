//! `--tls-client-crl FILE` (#1491, refs #108, #157): a certificate revocation list for client certificates, read at start.
//! A client certificate whose serial the CRL lists is refused at the handshake (no byte of HTTP, on any route, the
//! WebSocket route included: there is one listener and one verifier); one that is not listed connects; a CRL file that
//! is missing, unreadable, not PEM, empty, holds no CRL, is a link or is writable by group or other stops the server
//! before it listens; the option needs `--tls-client-ca`. Policy (see `src/tls.rs`): a CRL that has expired is
//! ENFORCED (every certificate of its issuer is refused until the file is replaced and the server restarted) and the log
//! says so; a status that no configured CRL can answer (another issuer's certificate) is allowed. Certificates and CRLs
//! are made at test time, dates relative to now (rcgen). Reload without a restart is #1492 and is not tested here.

mod tls_common;

use std::path::PathBuf;
use std::time::{Duration, Instant};
use tls_common::*;

const REVOKED: u64 = 4242;
const GOOD: u64 = 4243;

/// A server that requires client certificates from `client_ca` and checks them against `crl`.
struct Crl {
    dir: Dir,
    server_ca: Ca,
    client_ca: Ca,
    files: Files,
    crl: PathBuf,
}

fn crl_setup(revoked: &[u64], expired: bool) -> Crl {
    let dir = Dir::new();
    let (server_ca, client_ca) = (Ca::new("server ca"), Ca::new("client ca"));
    let files = server_files(
        &dir,
        &server_ca.issue("klatch", Use::Server),
        &client_ca,
        0o600,
    );
    let crl = dir.0.join("pki/client.crl.pem");
    write_file(&crl, &client_ca.crl_pem(revoked, expired), 0o644);
    Crl {
        dir,
        server_ca,
        client_ca,
        files,
        crl,
    }
}

fn crl_args(c: &Crl) -> Vec<String> {
    let mut args = tls_args(&c.files);
    args.extend(["--tls-client-crl".into(), c.crl.display().to_string()]);
    args
}

fn listening(c: &Crl) -> Server {
    Server::started(&c.dir, "https", &crl_args(c))
}

fn refused(c: &Crl, args: &[String], needle: &str) {
    let mut all = vec!["--listen".to_string(), "127.0.0.1:0".to_string()];
    all.extend_from_slice(args);
    let (code, err) = refused_start(&c.dir, "https", &all);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains(needle), "wanted {needle:?} in: {err}");
    assert!(!err.contains("listening on"), "{err}");
    assert!(
        !err.contains("PRIVATE KEY"),
        "a message holds key material: {err}"
    );
    // a fresh log for the next refusal in the same test
    let _ = std::fs::remove_file(c.dir.0.join("err.log"));
}

fn wait_for_log(c: &Crl, needle: &str) -> String {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let log = c.dir.log();
        if log.contains(needle) || Instant::now() > until {
            return log;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_revoked_client_certificate_is_refused_at_the_handshake_and_an_unlisted_one_connects() {
    let c = crl_setup(&[REVOKED], false);
    let server = listening(&c);
    let bad = c
        .client_ca
        .issue_serial("worker-1", Use::Client, Some(REVOKED));
    let good = c
        .client_ca
        .issue_serial("worker-2", Use::Client, Some(GOOD));
    let bad_config = client_config(&c.server_ca, Some(&bad), None);
    let good_config = client_config(&c.server_ca, Some(&good), None);
    // every route, the WebSocket route included, is behind the same handshake
    for path in ["/v1/stream", "/v1/frames", "/mcp", "/v1/ws", "/nope"] {
        let err = get_status(server.addr, &bad_config, path)
            .expect_err("a revoked certificate got an HTTP answer");
        // what the revoked client sees: a fatal TLS alert, no HTTP
        assert!(
            err.to_string().contains("CertificateRevoked"),
            "{path}: {err}"
        );
    }
    assert_eq!(get_status(server.addr, &good_config, "/nope").unwrap(), 404);
    assert_eq!(
        get_status(server.addr, &good_config, "/v1/stream").unwrap(),
        401
    );
    // the refusal is named in the server log, with no certificate content
    let log = wait_for_log(&c, "revoked");
    assert!(log.contains("client certificate refused: revoked"), "{log}");
    assert!(!log.contains("worker-1"), "{log}");
}

#[test]
fn the_start_line_says_the_crl_is_in_force_and_that_an_expired_one_locks_clients_out() {
    let c = crl_setup(&[REVOKED], false);
    let _server = listening(&c);
    let log = c.dir.log();
    assert!(log.contains("client certificate revocation: CRL"), "{log}");
    assert!(log.contains("expiry is enforced"), "{log}");
}

#[test]
fn an_expired_crl_refuses_every_certificate_of_its_issuer_and_the_log_says_why() {
    let c = crl_setup(&[REVOKED], true);
    let server = listening(&c);
    let good = c
        .client_ca
        .issue_serial("worker-2", Use::Client, Some(GOOD));
    let config = client_config(&c.server_ca, Some(&good), None);
    let err = get_status(server.addr, &config, "/v1/stream")
        .expect_err("an expired CRL must refuse, not fall open");
    assert!(err.to_string().contains("alert"), "{err}");
    let log = wait_for_log(&c, "expired");
    assert!(
        log.contains("client certificate refused: the CRL has expired")
            && log.contains("replace the CRL file and restart"),
        "{log}"
    );
}

#[test]
fn a_certificate_of_an_issuer_no_crl_covers_is_allowed_as_before() {
    // the CRL is by another CA: it says nothing about this CA's certificates, so their status is unknown and the policy
    // (unknown status allowed) lets them in; the documented hazard is a CRL by the wrong CA, which does nothing
    let c = crl_setup(&[], false);
    let other = Ca::new("another ca");
    write_file(
        &c.dir.0.join("pki/other.crl"),
        &other.crl_pem(&[REVOKED], false),
        0o644,
    );
    let mut args = tls_args(&c.files);
    args.extend([
        "--tls-client-crl".into(),
        c.dir.0.join("pki/other.crl").display().to_string(),
    ]);
    let server = Server::started(&c.dir, "https", &args);
    let leaf = c
        .client_ca
        .issue_serial("worker-1", Use::Client, Some(REVOKED));
    let config = client_config(&c.server_ca, Some(&leaf), None);
    assert_eq!(get_status(server.addr, &config, "/nope").unwrap(), 404);
}

#[test]
fn a_crl_file_that_cannot_be_used_stops_the_server_before_it_listens() {
    use std::os::unix::fs::PermissionsExt;
    let c = crl_setup(&[REVOKED], false);
    let with = |path: &std::path::Path| {
        let mut a = tls_args(&c.files);
        a.extend(["--tls-client-crl".into(), path.display().to_string()]);
        a
    };
    // missing
    refused(&c, &with(&c.dir.0.join("pki/none.crl")), "client CRL file");
    // not PEM
    let junk = c.dir.0.join("pki/junk.crl");
    write_file(&junk, "this is not a crl\n", 0o644);
    refused(&c, &with(&junk), "holds no CRL");
    // empty
    let empty = c.dir.0.join("pki/empty.crl");
    write_file(&empty, "", 0o644);
    refused(&c, &with(&empty), "is empty");
    // PEM of something else (a certificate)
    let cert = c.dir.0.join("pki/cert.crl");
    write_file(&cert, &c.client_ca.pem, 0o644);
    refused(&c, &with(&cert), "holds no CRL");
    // a damaged PEM block
    let broken = c.dir.0.join("pki/broken.crl");
    let pem = c.client_ca.crl_pem(&[REVOKED], false);
    write_file(&broken, &pem[..pem.len() / 2], 0o644);
    refused(&c, &with(&broken), "client CRL file");
    // writable by group, then by other
    for mode in [0o664, 0o646] {
        std::fs::set_permissions(&c.crl, std::fs::Permissions::from_mode(mode)).unwrap();
        refused(&c, &crl_args(&c), "writable");
    }
    // a symbolic link is not the file
    std::fs::set_permissions(&c.crl, std::fs::Permissions::from_mode(0o644)).unwrap();
    let link = c.dir.0.join("pki/link.crl");
    std::os::unix::fs::symlink(&c.crl, &link).unwrap();
    refused(&c, &with(&link), "symbolic link");
    // a directory
    refused(&c, &with(&c.dir.0.join("pki")), "not a regular file");
    // and the good file still starts the server
    let _ = Server::started(&c.dir, "https", &crl_args(&c));
}

#[test]
fn a_crl_needs_a_client_ca() {
    let c = crl_setup(&[REVOKED], false);
    let mut args = tls_args(&c.files);
    let at = args.iter().position(|a| a == "--tls-client-ca").unwrap();
    args.drain(at..at + 2);
    args.extend(["--tls-client-crl".into(), c.crl.display().to_string()]);
    refused(&c, &args, "--tls-client-crl needs --tls-client-ca");
}

#[test]
fn a_crl_option_without_a_value_or_twice_is_refused() {
    let c = crl_setup(&[REVOKED], false);
    let mut args = tls_args(&c.files);
    args.push("--tls-client-crl".into());
    refused(&c, &args, "--tls-client-crl needs a value");
    let mut twice = crl_args(&c);
    twice.extend(["--tls-client-crl".into(), c.crl.display().to_string()]);
    refused(&c, &twice, "more than once");
}

// ---------------------------------------------------------------------------------------------------------------
// review of #1617: silent lock-outs, a second CRL of one issuer, the file's size, the log's rate

const UNUSABLE: &str = "client certificate refused: the CRL is not usable";

#[test]
fn a_crl_with_a_bad_signature_locks_its_issuers_clients_out_and_the_log_says_so() {
    // a CRL that names the CA as issuer but was not signed by it: every certificate of that CA is refused, so the
    // operator must be told which file to look at (the lock-out was silent: a handshake failure and no line)
    let c = crl_setup(&[], false);
    write_file(
        &c.dir.0.join("pki/forged.crl"),
        &c.client_ca.forged_crl_pem(&[REVOKED]),
        0o644,
    );
    let mut args = tls_args(&c.files);
    args.extend([
        "--tls-client-crl".into(),
        c.dir.0.join("pki/forged.crl").display().to_string(),
    ]);
    let server = Server::started(&c.dir, "https", &args);
    let good = c
        .client_ca
        .issue_serial("worker-2", Use::Client, Some(GOOD));
    let config = client_config(&c.server_ca, Some(&good), None);
    get_status(server.addr, &config, "/v1/stream")
        .expect_err("a forged CRL must refuse, not fall open");
    let log = wait_for_log(&c, UNUSABLE);
    assert!(
        log.contains(UNUSABLE) && log.contains("replace the CRL file and restart"),
        "{log}"
    );
    assert!(!log.contains("worker-2"), "{log}");
}

/// A server whose client CA file holds a root, with clients of an intermediate CA that has `usages`, and a CRL signed by
/// that intermediate (claiming cRLSign, as `Ca::crl_pem` does for a CA without it).
fn intermediate_setup(usages: Vec<rcgen::KeyUsagePurpose>, revoked: &[u64]) -> (Crl, Ca, Server) {
    let mut c = crl_setup(&[], false);
    let intermediate = c.client_ca.intermediate("client intermediate", usages);
    c.crl = c.dir.0.join("pki/intermediate.crl.pem");
    write_file(&c.crl, &intermediate.crl_pem(revoked, false), 0o644);
    let server = listening(&c);
    (c, intermediate, server)
}

#[test]
fn an_intermediate_ca_that_may_sign_crls_works_like_a_root_for_revocation() {
    use rcgen::KeyUsagePurpose::{CrlSign, KeyCertSign};
    let (c, inter, server) = intermediate_setup(vec![KeyCertSign, CrlSign], &[REVOKED]);
    let good = inter.issue_serial("worker-2", Use::Client, Some(GOOD));
    let bad = inter.issue_serial("worker-1", Use::Client, Some(REVOKED));
    let ok = client_config(&c.server_ca, Some(&good), None);
    assert_eq!(get_status(server.addr, &ok, "/nope").unwrap(), 404);
    let no = client_config(&c.server_ca, Some(&bad), None);
    let err = get_status(server.addr, &no, "/nope").expect_err("revoked");
    assert!(err.to_string().contains("CertificateRevoked"), "{err}");
}

#[test]
fn an_intermediate_ca_that_may_not_sign_crls_locks_its_clients_out_and_the_log_says_so() {
    use rcgen::KeyUsagePurpose::KeyCertSign;
    let (c, inter, server) = intermediate_setup(vec![KeyCertSign], &[]);
    let good = inter.issue_serial("worker-2", Use::Client, Some(GOOD));
    let config = client_config(&c.server_ca, Some(&good), None);
    get_status(server.addr, &config, "/v1/stream")
        .expect_err("no cRLSign: the CRL cannot be trusted");
    let log = wait_for_log(&c, UNUSABLE);
    assert!(log.contains(UNUSABLE), "{log}");
}

#[test]
fn two_crls_of_one_issuer_are_refused_at_start_because_the_first_would_win() {
    // `cat new.crl >> client.crl`: the verifier uses the first CRL of an issuer, so an older empty one before a newer one
    // that lists a serial would accept the revoked certificate; one CRL per issuer, replace the file, never append
    let c = crl_setup(&[], false);
    let both = format!(
        "{}{}",
        c.client_ca.crl_pem(&[], false),
        c.client_ca.crl_pem(&[REVOKED], false)
    );
    std::fs::remove_file(&c.crl).unwrap();
    write_file(&c.crl, &both, 0o644);
    refused(&c, &crl_args(&c), "one CRL per issuer");
    // and the order does not matter, nor does expiry
    let both = format!(
        "{}{}",
        c.client_ca.crl_pem(&[REVOKED], true),
        c.client_ca.crl_pem(&[], false)
    );
    std::fs::remove_file(&c.crl).unwrap();
    write_file(&c.crl, &both, 0o644);
    refused(&c, &crl_args(&c), "replace the file, never append");
}

#[test]
fn crls_of_different_issuers_may_share_a_file() {
    let c = crl_setup(&[], false);
    let other = Ca::new("another ca");
    let both = format!(
        "{}{}",
        c.client_ca.crl_pem(&[REVOKED], false),
        other.crl_pem(&[], false)
    );
    std::fs::remove_file(&c.crl).unwrap();
    write_file(&c.crl, &both, 0o644);
    let server = listening(&c);
    let bad = c
        .client_ca
        .issue_serial("worker-1", Use::Client, Some(REVOKED));
    let config = client_config(&c.server_ca, Some(&bad), None);
    get_status(server.addr, &config, "/nope").expect_err("revoked");
}

#[test]
fn a_file_over_the_size_cap_or_with_too_many_crls_is_refused_before_it_is_parsed() {
    use std::os::unix::fs::PermissionsExt;
    let c = crl_setup(&[REVOKED], false);
    let with = |path: &std::path::Path| {
        let mut a = tls_args(&c.files);
        a.extend(["--tls-client-crl".into(), path.display().to_string()]);
        a
    };
    // a sparse file of 5 MiB: refused by its size, so the server never reads it into memory
    let big = c.dir.0.join("pki/big.crl");
    let f = std::fs::File::create(&big).unwrap();
    f.set_len(5 * 1024 * 1024).unwrap();
    std::fs::set_permissions(&big, std::fs::Permissions::from_mode(0o644)).unwrap();
    refused(&c, &with(&big), "larger than 4 MiB");
    // seventeen blocks (the same CRL: the count is checked first)
    let many = c.dir.0.join("pki/many.crl");
    write_file(
        &many,
        &c.client_ca.crl_pem(&[REVOKED], false).repeat(17),
        0o644,
    );
    refused(&c, &with(&many), "at most 16");
}

#[test]
fn many_revoked_handshakes_make_one_log_line_not_one_per_handshake() {
    let c = crl_setup(&[REVOKED], false);
    let server = listening(&c);
    let bad = c
        .client_ca
        .issue_serial("worker-1", Use::Client, Some(REVOKED));
    let config = client_config(&c.server_ca, Some(&bad), None);
    for _ in 0..30 {
        get_status(server.addr, &config, "/nope").expect_err("revoked");
    }
    wait_for_log(&c, "client certificate refused: revoked");
    std::thread::sleep(Duration::from_millis(300));
    let lines = c
        .dir
        .log()
        .lines()
        .filter(|l| l.contains("client certificate refused: revoked"))
        .count();
    assert!((1..=2).contains(&lines), "{lines} lines for 30 handshakes");
}
