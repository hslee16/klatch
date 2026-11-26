//! Pins of the client CRL (#1491) for what the earlier tests checked by hand only (#1618, #1622): the refusal of a revoked
//! client over TLS 1.2, the alert of an expired CRL, chains with an intermediate CA, the trust anchor that is never
//! checked, and the error of a CRL the verifier cannot build on. Certificates and CRLs are made at test time, dates relative
//! to now. Every connect and read has a deadline (`tls_connect` sets 20 s), so a regression fails an assertion.

mod tls_common;

use rcgen::KeyUsagePurpose::{CrlSign, KeyCertSign};
use std::net::SocketAddr;
use tls_common::der::*;
use tls_common::*;

const REVOKED: u64 = 4242;
const GOOD: u64 = 4243;

struct S {
    dir: Dir,
    files: Files,
    server_ca: Ca,
    client_ca: Ca,
    crl: std::path::PathBuf,
}

/// A server whose client CA file holds `client_ca` (the root, unless a test rewrites the file) and a CRL file the test fills.
fn setup_with(client_ca: Ca) -> S {
    let dir = Dir::new();
    let server_ca = Ca::new("server ca");
    let files = server_files(
        &dir,
        &server_ca.issue("klatch", Use::Server),
        &client_ca,
        0o600,
    );
    let crl = dir.0.join("pki/client.crl.pem");
    S {
        dir,
        files,
        server_ca,
        client_ca,
        crl,
    }
}

fn setup() -> S {
    setup_with(Ca::new("client ca"))
}

fn args(s: &S) -> Vec<String> {
    let mut a = tls_args(&s.files);
    a.extend(["--tls-client-crl".into(), s.crl.display().to_string()]);
    a
}

fn start(s: &S) -> Server {
    Server::started(&s.dir, "https", &args(s))
}

/// What a client with `leaf` gets: the HTTP status of a request for a path that does not exist (404: let in), or the text of
/// the handshake error.
fn outcome(
    s: &S,
    addr: SocketAddr,
    leaf: &Leaf,
    versions: Option<&[&'static rustls::SupportedProtocolVersion]>,
) -> Result<u16, String> {
    let config = client_config(&s.server_ca, Some(leaf), versions);
    get_status(addr, &config, "/nope").map_err(|e| e.to_string())
}

const TLS12: &[&rustls::SupportedProtocolVersion] = &[&rustls::version::TLS12];

fn wait_log(s: &S, needle: &str) -> String {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let log = s.dir.log();
        if log.contains(needle) || std::time::Instant::now() > until {
            return log;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn over_tls_1_2_a_revoked_client_is_refused_with_the_revoked_alert_and_a_listed_one_only() {
    let s = setup();
    write_file(&s.crl, &s.client_ca.crl_pem(&[REVOKED], false), 0o644);
    let server = start(&s);
    let bad = s.client_ca.issue_serial("w1", Use::Client, Some(REVOKED));
    let good = s.client_ca.issue_serial("w2", Use::Client, Some(GOOD));
    let err = outcome(&s, server.addr, &bad, Some(TLS12))
        .expect_err("a revoked certificate got in over TLS 1.2");
    assert!(err.contains("CertificateRevoked"), "{err}");
    // the same client config is not vacuous: an unlisted certificate gets in over 1.2
    assert_eq!(outcome(&s, server.addr, &good, Some(TLS12)), Ok(404));
    let log = wait_log(&s, "client certificate refused: revoked");
    assert!(log.contains("client certificate refused: revoked"), "{log}");
}

#[test]
fn an_expired_crl_is_refused_with_the_unknown_ca_alert_and_not_a_revocation_alert_on_both_tls_versions()
 {
    let s = setup();
    write_file(&s.crl, &s.client_ca.crl_pem(&[REVOKED], true), 0o644);
    let server = start(&s);
    let good = s.client_ca.issue_serial("w2", Use::Client, Some(GOOD));
    for versions in [None, Some(TLS12)] {
        let err =
            outcome(&s, server.addr, &good, versions).expect_err("an expired CRL must refuse");
        assert!(err.contains("UnknownCA"), "{versions:?}: {err}");
        assert!(!err.contains("CertificateRevoked"), "{versions:?}: {err}");
    }
}

/// A root, an intermediate under it (serial 777), and the intermediate's leaves.
fn chain() -> (S, Ca) {
    let s = setup();
    let inter = s.client_ca.intermediate_serial(
        "client intermediate",
        vec![KeyCertSign, CrlSign],
        Some(777),
    );
    (s, inter)
}

#[test]
fn a_crl_of_the_root_that_lists_the_intermediate_refuses_every_leaf_under_it_on_both_versions() {
    let (s, inter) = chain();
    write_file(&s.crl, &s.client_ca.crl_pem(&[777], false), 0o644);
    let server = start(&s);
    let (a, b) = (
        inter.issue_serial("w1", Use::Client, Some(GOOD)),
        inter.issue_serial("w2", Use::Client, Some(GOOD + 1)),
    );
    for leaf in [&a, &b] {
        for versions in [None, Some(TLS12)] {
            let err = outcome(&s, server.addr, leaf, versions)
                .expect_err("a leaf of a revoked intermediate got in");
            assert!(err.contains("CertificateRevoked"), "{versions:?}: {err}");
        }
    }
    // a leaf the root itself signed is not touched by the revocation of the intermediate
    let direct = s.client_ca.issue_serial("w3", Use::Client, Some(GOOD + 2));
    assert_eq!(outcome(&s, server.addr, &direct, None), Ok(404));
}

#[test]
fn a_crl_of_the_root_alone_leaves_the_leaves_of_an_intermediate_allowed_whatever_serials_it_lists()
{
    let (s, inter) = chain();
    // the root lists a serial that one leaf of the intermediate also has: the CRL is the root's and covers what the root
    // issued, not what the intermediate did
    write_file(&s.crl, &s.client_ca.crl_pem(&[REVOKED, 9999], false), 0o644);
    let server = start(&s);
    let same_serial = inter.issue_serial("w1", Use::Client, Some(REVOKED));
    let other = inter.issue_serial("w2", Use::Client, Some(GOOD));
    for leaf in [&same_serial, &other] {
        for versions in [None, Some(TLS12)] {
            assert_eq!(outcome(&s, server.addr, leaf, versions), Ok(404));
        }
    }
    // and the same CRL does revoke a leaf the root issued, so that it is a CRL in force
    let direct = s.client_ca.issue_serial("w3", Use::Client, Some(REVOKED));
    let err = outcome(&s, server.addr, &direct, None).expect_err("the root's own leaf is listed");
    assert!(err.contains("CertificateRevoked"), "{err}");
}

#[test]
fn the_crl_of_an_intermediate_revokes_its_listed_leaf_only_and_a_root_crl_beside_it_changes_nothing()
 {
    let (s, inter) = chain();
    let both = format!(
        "{}{}",
        s.client_ca.crl_pem(&[], false),
        inter.crl_pem(&[REVOKED], false)
    );
    write_file(&s.crl, &both, 0o644);
    let server = start(&s);
    let bad = inter.issue_serial("w1", Use::Client, Some(REVOKED));
    let good = inter.issue_serial("w2", Use::Client, Some(GOOD));
    for versions in [None, Some(TLS12)] {
        let err = outcome(&s, server.addr, &bad, versions).expect_err("listed by its issuer");
        assert!(err.contains("CertificateRevoked"), "{versions:?}: {err}");
        assert_eq!(outcome(&s, server.addr, &good, versions), Ok(404));
    }
}

#[test]
fn the_trust_anchor_is_never_checked_even_when_a_crl_lists_it() {
    // the root's own serial is on its own CRL: the anchor is not part of the chain that is checked, so every leaf under
    // it, direct or through an intermediate, is still let in
    let s = setup_with(Ca::new_serial("client ca", Some(555)));
    let inter = s.client_ca.intermediate_serial(
        "client intermediate",
        vec![KeyCertSign, CrlSign],
        Some(777),
    );
    write_file(&s.crl, &s.client_ca.crl_pem(&[555], false), 0o644);
    let server = start(&s);
    let direct = s.client_ca.issue_serial("w1", Use::Client, Some(GOOD));
    let through = inter.issue_serial("w2", Use::Client, Some(GOOD));
    for versions in [None, Some(TLS12)] {
        assert_eq!(outcome(&s, server.addr, &direct, versions), Ok(404));
        assert_eq!(outcome(&s, server.addr, &through, versions), Ok(404));
    }
}

#[test]
fn an_intermediate_that_is_the_anchor_is_not_checked_either_whatever_the_roots_crl_says() {
    // the client CA file holds the intermediate itself: it is the anchor, so a root CRL that lists it does not refuse its
    // leaves (the root is not in this server's trust at all, and the CRL is of an issuer that is no CA of the file)
    let (s, inter) = chain();
    rewrite_file(&s.files.ca, &inter.pem, 0o644);
    write_file(&s.crl, &s.client_ca.crl_pem(&[777], false), 0o644);
    let server = start(&s);
    let leaf = inter.issue_serial("w1", Use::Client, Some(GOOD));
    assert_eq!(outcome(&s, server.addr, &leaf, None), Ok(404));
}

#[test]
fn a_crl_the_verifier_cannot_build_on_stops_the_server_at_start_in_words_that_name_the_file() {
    let s = setup();
    // a CRL that has an issuer and times (the start-up reads pass) but not a signature the verifier can parse
    write_file(
        &s.crl,
        &with_junk_signature(&s.client_ca.crl_pem(&[REVOKED], false)),
        0o644,
    );
    let mut all = vec!["--listen".to_string(), "127.0.0.1:0".to_string()];
    all.extend(args(&s));
    let (code, err) = refused_start(&s.dir, "https", &all);
    assert_eq!(code, Some(1), "{err}");
    assert!(
        err.contains("cannot build the client certificate check from the client CRL file"),
        "{err}"
    );
    assert!(err.contains(&s.crl.display().to_string()), "{err}");
    assert!(
        !err.contains("listening on") && !err.contains("PRIVATE KEY"),
        "{err}"
    );
    assert!(!err.contains("client ca"), "{err}");
}

#[test]
fn a_file_over_the_cap_that_is_not_pem_is_refused_by_its_size_before_it_is_read() {
    use std::os::unix::fs::PermissionsExt;
    let s = setup();
    // 5 MiB of text that is not PEM: read at all, it would be refused as "not valid PEM" or as grown while read, so the
    // message names the check that ran first (the one at the metadata step)
    let big = s.dir.0.join("pki/big.crl");
    std::fs::write(&big, vec![b'x'; 5 * 1024 * 1024]).unwrap();
    std::fs::set_permissions(&big, std::fs::Permissions::from_mode(0o644)).unwrap();
    let mut all = vec!["--listen".to_string(), "127.0.0.1:0".to_string()];
    all.extend(tls_args(&s.files));
    all.extend(["--tls-client-crl".into(), big.display().to_string()]);
    let (code, err) = refused_start(&s.dir, "https", &all);
    assert_eq!(code, Some(1), "{err}");
    assert!(
        err.contains("is larger than 4 MiB: it is not read"),
        "{err}"
    );
    assert!(
        !err.contains("while it was being read") && !err.contains("not valid PEM"),
        "{err}"
    );
}
