//! `--tls-client-crl` start-up checks of #1614 (refs #1491, #108): at start the server reads each CRL's issuer and times.
//! REFUSED: a CRL whose issuer or times cannot be read, whose `nextUpdate` is missing (the verifier cannot parse such a
//! CRL either), or whose times are before 1970. WARNED (the server starts): a CRL whose issuer is no CA of the client CA
//! file (it still applies to clients that present an intermediate CA with that name), one past its `nextUpdate`, one due
//! within seven days. Certificates and CRLs are made at test time, every date relative to now (rcgen); the reader's own
//! edge cases are unit tests in `src/der_tests.rs`, and the pure decision is in `src/tls_crl.rs`.

mod tls_common;

use std::path::PathBuf;
use tls_common::der::*;
use tls_common::*;

const DAY: i64 = 86_400;

struct Setup {
    dir: Dir,
    files: Files,
    server_ca: Ca,
    client_ca: Ca,
    crl: PathBuf,
}

/// A server whose client CA file holds `client_ca` only, and a CRL file the test fills.
fn setup() -> Setup {
    let dir = Dir::new();
    let (server_ca, client_ca) = (Ca::new("server ca"), Ca::new("client ca"));
    let files = server_files(
        &dir,
        &server_ca.issue("klatch", Use::Server),
        &client_ca,
        0o600,
    );
    let crl = dir.0.join("pki/client.crl.pem");
    Setup {
        dir,
        files,
        server_ca,
        client_ca,
        crl,
    }
}

fn args(s: &Setup) -> Vec<String> {
    let mut a = tls_args(&s.files);
    a.extend(["--tls-client-crl".into(), s.crl.display().to_string()]);
    a
}

/// The server is refused at start, with `needle` in its message and no listener, and the message holds none of `hidden`.
fn refused(s: &Setup, needle: &str, hidden: &[&str]) {
    let mut all = vec!["--listen".to_string(), "127.0.0.1:0".to_string()];
    all.extend(args(s));
    let (code, err) = refused_start(&s.dir, "https", &all);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains(needle), "wanted {needle:?} in: {err}");
    assert!(!err.contains("listening on"), "{err}");
    for h in hidden {
        assert!(!err.contains(h), "the message holds {h:?}: {err}");
    }
}

fn started(s: &Setup) -> (Server, String) {
    let server = Server::started(&s.dir, "https", &args(s));
    let log = s.dir.log();
    (server, log)
}

const NO_CA: &str = "matches no CA in the client CA file; it only applies to clients that present an intermediate CA with that name; if that is not intended it does nothing";

#[test]
fn a_crl_whose_issuer_is_in_no_ca_of_the_file_starts_with_a_warning_that_names_no_one() {
    let s = setup();
    let stranger = Ca::new("stranger ca");
    write_file(&s.crl, &stranger.crl_pem(&[4242], false), 0o644);
    let (_server, log) = started(&s);
    assert!(
        log.contains(&format!("warning: CRL 1 of 1 {NO_CA}")),
        "{log}"
    );
    assert!(
        !log.contains("stranger ca") && !log.contains("4242"),
        "{log}"
    );
}

#[test]
fn a_matching_and_a_stranger_crl_in_one_file_warn_for_the_stranger_alone() {
    let s = setup();
    let stranger = Ca::new("stranger ca");
    let both = format!(
        "{}{}",
        s.client_ca.crl_pem(&[], false),
        stranger.crl_pem(&[], false)
    );
    write_file(&s.crl, &both, 0o644);
    let (_server, log) = started(&s);
    assert!(
        log.contains(&format!("warning: CRL 2 of 2 {NO_CA}")),
        "{log}"
    );
    assert!(!log.contains("CRL 1 of 2"), "{log}");
}

#[test]
fn a_crl_whose_issuer_is_a_ca_of_the_file_starts_without_a_warning() {
    let s = setup();
    write_file(&s.crl, &s.client_ca.crl_pem(&[], false), 0o644);
    let (_server, log) = started(&s);
    assert!(log.contains("client certificate revocation: CRL"), "{log}");
    assert!(!log.contains("warning:"), "{log}");
}

#[test]
fn the_crl_of_an_intermediate_with_only_the_root_in_the_ca_file_starts_warns_and_revokes() {
    use rcgen::KeyUsagePurpose::{CrlSign, KeyCertSign};
    let s = setup();
    let inter = s
        .client_ca
        .intermediate("client intermediate", vec![KeyCertSign, CrlSign]);
    write_file(&s.crl, &inter.crl_pem(&[4242], false), 0o644);
    let (server, log) = started(&s);
    assert!(
        log.contains(&format!("warning: CRL 1 of 1 {NO_CA}")),
        "{log}"
    );
    // and it is in force for the clients of that intermediate (their chain reaches the root)
    let bad = inter.issue_serial("worker-1", Use::Client, Some(4242));
    let good = inter.issue_serial("worker-2", Use::Client, Some(4243));
    let no = client_config(&s.server_ca, Some(&bad), None);
    let err = get_status(server.addr, &no, "/nope").expect_err("revoked");
    assert!(err.to_string().contains("CertificateRevoked"), "{err}");
    let ok = client_config(&s.server_ca, Some(&good), None);
    assert_eq!(get_status(server.addr, &ok, "/nope").unwrap(), 404);
}

#[test]
fn the_roots_crl_that_revokes_an_intermediate_still_refuses_its_clients() {
    use rcgen::KeyUsagePurpose::{CrlSign, KeyCertSign};
    let s = setup();
    let inter = s.client_ca.intermediate_serial(
        "client intermediate",
        vec![KeyCertSign, CrlSign],
        Some(777),
    );
    write_file(&s.crl, &s.client_ca.crl_pem(&[777], false), 0o644);
    let (server, log) = started(&s);
    assert!(!log.contains("warning:"), "{log}");
    let leaf = inter.issue_serial("worker-1", Use::Client, Some(4243));
    let config = client_config(&s.server_ca, Some(&leaf), None);
    let err = get_status(server.addr, &config, "/nope").expect_err("the intermediate is revoked");
    assert!(err.to_string().contains("CertificateRevoked"), "{err}");
}

#[test]
fn the_crl_of_an_intermediate_works_when_only_the_intermediate_is_in_the_ca_file() {
    use rcgen::KeyUsagePurpose::{CrlSign, KeyCertSign};
    let s = setup();
    let inter = s
        .client_ca
        .intermediate("client intermediate", vec![KeyCertSign, CrlSign]);
    rewrite_file(&s.files.ca, &inter.pem, 0o644);
    write_file(&s.crl, &inter.crl_pem(&[4242], false), 0o644);
    let (server, log) = started(&s);
    assert!(!log.contains("warning:"), "{log}");
    let bad = inter.issue_serial("worker-1", Use::Client, Some(4242));
    let no = client_config(&s.server_ca, Some(&bad), None);
    get_status(server.addr, &no, "/nope").expect_err("revoked");
}

#[test]
fn a_crl_that_is_past_its_next_update_warns_and_the_server_still_starts() {
    let s = setup();
    write_file(&s.crl, &s.client_ca.crl_pem_due(-DAY), 0o644);
    let (_server, log) = started(&s);
    assert!(
        log.contains("warning: CRL 1 of 1 has passed its nextUpdate"),
        "{log}"
    );
    assert!(
        log.contains("every client certificate of its issuer is refused"),
        "{log}"
    );
    assert!(!log.contains("client ca"), "{log}");
}

#[test]
fn a_crl_due_within_seven_days_warns_and_one_due_later_does_not() {
    for (due, warns) in [
        (3 * DAY, true),
        (6 * DAY, true),
        (8 * DAY, false),
        (30 * DAY, false),
    ] {
        let s = setup();
        write_file(&s.crl, &s.client_ca.crl_pem_due(due), 0o644);
        let (_server, log) = started(&s);
        assert_eq!(
            log.contains("warning: CRL 1 of 1 passes its nextUpdate within 7 days"),
            warns,
            "due in {due}s: {log}"
        );
        assert!(!log.contains("has passed its nextUpdate"), "{log}");
    }
}

#[test]
fn each_crl_of_a_file_is_judged_alone_and_the_warning_says_which() {
    let s = setup();
    let other = Ca::new("another ca");
    rewrite_file(
        &s.files.ca,
        &format!("{}{}", s.client_ca.pem, other.pem),
        0o644,
    );
    let both = format!(
        "{}{}",
        s.client_ca.crl_pem_due(30 * DAY),
        other.crl_pem_due(-DAY)
    );
    write_file(&s.crl, &both, 0o644);
    let (_server, log) = started(&s);
    assert!(
        log.contains("warning: CRL 2 of 2 has passed its nextUpdate"),
        "{log}"
    );
    assert!(!log.contains("CRL 1 of 2 has"), "{log}");
}

#[test]
fn sixteen_crls_of_sixteen_issuers_are_accepted() {
    // 16 distinct issuers is the most one file may hold (the 17th block is refused, see tls_crl.rs); the 16th is not
    let s = setup();
    let cas: Vec<Ca> = (0..16).map(|n| Ca::new(&format!("ca {n}"))).collect();
    let pems: String = cas.iter().map(|c| c.pem.clone()).collect();
    rewrite_file(&s.files.ca, &pems, 0o644);
    let crls: String = cas.iter().map(|c| c.crl_pem_due(30 * DAY)).collect();
    write_file(&s.crl, &crls, 0o644);
    let (_server, log) = started(&s);
    assert!(!log.contains("warning:"), "{log}");
}

// ---------------------------------------------------------------------------------------------------------------
// a CRL without a nextUpdate, and times before 1970: refused (the verifier cannot use them either)

/// The CRL of `pem` with its nextUpdate element removed (the signature no longer verifies; the start-up read does not
/// check it, which is the point: the server must refuse the file by its shape).
fn without_next_update(pem: &str) -> String {
    use rustls::pki_types::{CertificateRevocationListDer, pem::PemObject};
    let der = CertificateRevocationListDer::from_pem_slice(pem.as_bytes()).unwrap();
    let (_, list, _) = element(der.as_ref());
    let (_, tbs, after_tbs) = element(list);
    let mut kept = Vec::new();
    let mut rest = tbs;
    let mut seen_times = 0;
    while !rest.is_empty() {
        let (tag, content, next) = element(rest);
        let whole = &rest[..rest.len() - next.len()];
        if matches!(tag, 0x17 | 0x18) {
            seen_times += 1;
            if seen_times == 2 {
                rest = next;
                continue;
            }
        }
        let _ = content;
        kept.extend_from_slice(whole);
        rest = next;
    }
    assert_eq!(seen_times, 2, "rcgen wrote thisUpdate and nextUpdate");
    let mut list = wrap(0x30, &kept);
    list.extend_from_slice(after_tbs);
    pem_of(&wrap(0x30, &list))
}

#[test]
fn a_crl_without_a_next_update_is_refused_at_start_in_words() {
    let s = setup();
    write_file(
        &s.crl,
        &without_next_update(&s.client_ca.crl_pem(&[], false)),
        0o644,
    );
    refused(&s, "has no nextUpdate", &["client ca"]);
    // and the message says why and what to do, and names the CRL
    let mut all = vec!["--listen".to_string(), "127.0.0.1:0".to_string()];
    all.extend(args(&s));
    let (_, err) = refused_start(&s.dir, "https", &all);
    assert!(
        err.contains("CRL (number 1)") && err.contains("not a valid CRL for this server"),
        "{err}"
    );
}

#[test]
fn a_crl_with_times_before_1970_is_refused_at_start_in_words() {
    use std::time::{Duration, UNIX_EPOCH};
    let s = setup();
    let year = Duration::from_secs(365 * 86_400);
    write_file(
        &s.crl,
        &s.client_ca
            .crl_pem_between(UNIX_EPOCH - 2 * year, UNIX_EPOCH - year),
        0o644,
    );
    refused(&s, "before 1970", &["client ca"]);
}
