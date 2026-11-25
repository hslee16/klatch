//! `--tls-client-crl` start-up checks of #1614 (refs #1491, #108): at start the server compares each CRL's issuer with the
//! subjects of the client CA file and reads each CRL's `nextUpdate`. A CRL whose issuer no CA of the file has is REFUSED
//! (it would silently do nothing); a CRL that is past its `nextUpdate`, due within seven days, or without a `nextUpdate`
//! only WARNS (the server starts: an expired CRL is enforced, as documented). Certificates and CRLs are made at test time,
//! every date relative to now (rcgen); the reader's own edge cases (time encodings, truncation, lying lengths, a bounded
//! random run) are unit tests in `src/der_tests.rs`, and the pure decision is in `src/tls_crl.rs`.

mod tls_common;

use std::path::PathBuf;
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

#[test]
fn a_crl_whose_issuer_is_in_no_ca_of_the_file_is_refused_at_start() {
    let s = setup();
    let stranger = Ca::new("stranger ca");
    write_file(&s.crl, &stranger.crl_pem(&[4242], false), 0o644);
    refused(
        &s,
        "matches no CA certificate in the client CA file",
        &["stranger ca", "4242"],
    );
}

#[test]
fn one_matching_and_one_stranger_crl_refuse_and_the_message_names_the_file_not_the_content() {
    let s = setup();
    let stranger = Ca::new("stranger ca");
    let both = format!(
        "{}{}",
        s.client_ca.crl_pem(&[], false),
        stranger.crl_pem(&[], false)
    );
    write_file(&s.crl, &both, 0o644);
    refused(
        &s,
        &format!("the client CRL file {}", s.crl.display()),
        &["stranger ca", "client ca"],
    );
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
    // and it is in force: the revoked certificate of the intermediate is refused, an unlisted one connects
    let bad = inter.issue_serial("worker-1", Use::Client, Some(4242));
    let good = inter.issue_serial("worker-2", Use::Client, Some(4243));
    let no = client_config(&s.server_ca, Some(&bad), None);
    get_status(server.addr, &no, "/nope").expect_err("revoked");
    let ok = client_config(&s.server_ca, Some(&good), None);
    assert_eq!(get_status(server.addr, &ok, "/nope").unwrap(), 404);
}

#[test]
fn an_intermediate_crl_with_only_the_root_in_the_ca_file_is_refused_a_documented_limit() {
    // the CA file names no CA whose subject is the CRL's issuer, so the server cannot tell this CRL from a wrong one:
    // list the intermediate beside (or instead of) its root
    use rcgen::KeyUsagePurpose::{CrlSign, KeyCertSign};
    let s = setup();
    let inter = s
        .client_ca
        .intermediate("client intermediate", vec![KeyCertSign, CrlSign]);
    write_file(&s.crl, &inter.crl_pem(&[], false), 0o644);
    refused(&s, "matches no CA certificate", &[]);
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
    // the cap is on the count (16), and 16 distinct issuers is the most one file may hold: the 17th block is refused
    // (a test of that is in tls_crl.rs), the 16th is not
    let s = setup();
    let cas: Vec<Ca> = (0..16).map(|n| Ca::new(&format!("ca {n}"))).collect();
    let pems: String = cas.iter().map(|c| c.pem.clone()).collect();
    rewrite_file(&s.files.ca, &pems, 0o644);
    let crls: String = cas.iter().map(|c| c.crl_pem_due(30 * DAY)).collect();
    write_file(&s.crl, &crls, 0o644);
    let (_server, log) = started(&s);
    assert!(!log.contains("warning:"), "{log}");
}
