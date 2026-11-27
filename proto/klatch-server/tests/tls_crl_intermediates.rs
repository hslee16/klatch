//! `--tls-client-intermediates FILE` (#1623, refs #1614, #1491): the strict option. Certificates whose SUBJECT NAMES are
//! used to match the issuers of CRLs at start and at reload, and for nothing else: they are never trust anchors and
//! never used to build or verify a client's chain. With the flag a CRL whose issuer is neither a CA of the client CA file
//! nor a listed intermediate is refused at start; without it the behaviour of #1625 is unchanged (a warning). Every
//! certificate and CRL is made at test time with rcgen, dates relative to now. Reload is in `tls_crl_intermediates_reload.rs`.

mod tls_common;

use rcgen::KeyUsagePurpose::{CrlSign, KeyCertSign};
use std::path::PathBuf;
use tls_common::der::pem_of;
use tls_common::*;

struct Setup {
    dir: Dir,
    files: Files,
    server_ca: Ca,
    client_ca: Ca,
    inter: Ca,
    crl: PathBuf,
    inters: PathBuf,
}

/// Client CA file holds the ROOT only; an intermediate issues the clients; its certificate is in the intermediates file
/// and its CRL (revoking serial 4242) in the CRL file.
fn setup() -> Setup {
    let dir = Dir::new();
    let (server_ca, client_ca) = (Ca::new("server ca"), Ca::new("client ca"));
    let files = server_files(
        &dir,
        &server_ca.issue("klatch", Use::Server),
        &client_ca,
        0o600,
    );
    let inter = client_ca.intermediate("canary-inter-7731", vec![KeyCertSign, CrlSign]);
    let crl = dir.0.join("pki/client.crl.pem");
    let inters = dir.0.join("pki/intermediates.pem");
    write_file(&crl, &inter.crl_pem(&[4242], false), 0o644);
    write_file(&inters, &inter.pem, 0o644);
    Setup {
        dir,
        files,
        server_ca,
        client_ca,
        inter,
        crl,
        inters,
    }
}

fn args(s: &Setup, with_flag: bool) -> Vec<String> {
    let mut a = tls_args(&s.files);
    a.extend(["--tls-client-crl".into(), s.crl.display().to_string()]);
    if with_flag {
        a.extend([
            "--tls-client-intermediates".into(),
            s.inters.display().to_string(),
        ]);
    }
    a
}

/// A server start reads the FIRST "listening" line of the log and a refusal returns the whole log, so every run starts
/// from an empty one (a stale line of an earlier run in the same test would satisfy a check it must not).
fn fresh_log(dir: &Dir) {
    let _ = std::fs::remove_file(dir.0.join("err.log"));
}

fn started(s: &Setup, with_flag: bool) -> (Server, String) {
    fresh_log(&s.dir);
    let server = Server::started(&s.dir, "https", &args(s, with_flag));
    let log = s.dir.log();
    (server, log)
}

/// Run with exactly `args` (and `--listen`): refused at start with `needle`, no listener, none of `hidden` in the text.
fn refused_with(dir: &Dir, args: Vec<String>, needle: &str, hidden: &[&str]) {
    let mut all = vec!["--listen".to_string(), "127.0.0.1:0".to_string()];
    all.extend(args);
    fresh_log(dir);
    let (code, err) = refused_start(dir, "https", &all);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains(needle), "wanted {needle:?} in: {err}");
    assert!(!err.contains("listening on"), "{err}");
    for h in hidden {
        assert!(!err.contains(h), "the message holds {h:?}: {err}");
    }
}

fn refused(s: &Setup, needle: &str) {
    refused_with(&s.dir, args(s, true), needle, &["client ca", "stranger"]);
}

const NO_CA: &str = "matches no CA in the client CA file; it only applies to clients that present an intermediate CA with that name; if that is not intended it does nothing";

#[test]
fn with_the_flag_an_intermediate_crl_starts_without_a_warning_and_revokes_its_leaf() {
    let s = setup();
    let (server, log) = started(&s, true);
    assert!(log.contains("client certificate revocation: CRL"), "{log}");
    assert!(!log.contains("warning:"), "{log}");
    assert!(!log.contains("canary-inter-7731"), "{log}");
    let bad = s.inter.issue_serial("worker-1", Use::Client, Some(4242));
    let good = s.inter.issue_serial("worker-2", Use::Client, Some(4243));
    let no = client_config(&s.server_ca, Some(&bad), None);
    let err = get_status(server.addr, &no, "/nope").expect_err("revoked");
    assert!(err.to_string().contains("CertificateRevoked"), "{err}");
    let ok = client_config(&s.server_ca, Some(&good), None);
    assert_eq!(get_status(server.addr, &ok, "/nope").unwrap(), 404);
}

#[test]
fn without_the_flag_the_same_setup_starts_with_the_warning_as_before() {
    let s = setup();
    let (server, log) = started(&s, false);
    assert!(
        log.contains(&format!("warning: CRL 1 of 1 {NO_CA}")),
        "{log}"
    );
    let bad = s.inter.issue_serial("worker-1", Use::Client, Some(4242));
    let no = client_config(&s.server_ca, Some(&bad), None);
    get_status(server.addr, &no, "/nope").expect_err("revoked");
}

#[test]
fn with_the_flag_a_crl_of_a_trust_anchor_still_starts_and_so_does_a_mixed_file() {
    let s = setup();
    let both = format!(
        "{}{}",
        s.client_ca.crl_pem(&[], false),
        s.inter.crl_pem(&[4242], false)
    );
    rewrite_file(&s.crl, &both, 0o644);
    let (_server, log) = started(&s, true);
    assert!(!log.contains("warning:"), "{log}");
}

#[test]
fn with_the_flag_a_crl_matching_neither_a_ca_nor_an_intermediate_is_refused_at_start() {
    let s = setup();
    let stranger = Ca::new("stranger ca");
    rewrite_file(&s.crl, &stranger.crl_pem(&[4242], false), 0o644);
    refused(
        &s,
        "holds a CRL (number 1) whose issuer matches no CA in the client CA file and no certificate in the client intermediates file",
    );
    // a good CRL first and a stranger second: the second is the one named
    let both = format!(
        "{}{}",
        s.inter.crl_pem(&[], false),
        stranger.crl_pem(&[], false)
    );
    rewrite_file(&s.crl, &both, 0o644);
    refused(&s, "(number 2)");
    // the same stranger without the flag is only a warning
    let (_server, log) = started(&s, false);
    assert!(log.contains("warning: CRL 2 of 2"), "{log}");
}

#[test]
fn the_flag_needs_a_crl_a_client_ca_and_is_not_given_twice() {
    let s = setup();
    let tls = tls_args(&s.files);
    let flag = vec![
        "--tls-client-intermediates".to_string(),
        s.inters.display().to_string(),
    ];
    // no CRL: it would do nothing
    refused_with(
        &s.dir,
        [tls.clone(), flag.clone()].concat(),
        "--tls-client-intermediates needs --tls-client-crl: it only matches the issuers of CRLs, so without a CRL it would do nothing",
        &[],
    );
    // no client CA (the CA flag and its file removed)
    let no_ca: Vec<String> = [&tls[..4], &flag[..]].concat();
    let crl = vec!["--tls-client-crl".to_string(), s.crl.display().to_string()];
    refused_with(
        &s.dir,
        [no_ca, crl.clone()].concat(),
        "--tls-client-intermediates needs --tls-client-ca",
        &[],
    );
    // twice
    refused_with(
        &s.dir,
        [tls, crl, flag.clone(), flag].concat(),
        "--tls-client-intermediates is given more than once",
        &[],
    );
}

#[test]
fn the_intermediates_file_is_checked_like_the_ca_file() {
    let s = setup();
    // a symbolic link
    let link = s.dir.0.join("pki/link.pem");
    std::os::unix::fs::symlink(&s.inters, &link).unwrap();
    let mut a = args(&s, false);
    a.extend([
        "--tls-client-intermediates".into(),
        link.display().to_string(),
    ]);
    refused_with(&s.dir, a, "is a symbolic link", &["client ca"]);
    // a FIFO is refused at once, not waited on
    let fifo = s.dir.0.join("pki/fifo.pem");
    let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    // SAFETY: mkfifo gets a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    let mut a = args(&s, false);
    a.extend([
        "--tls-client-intermediates".into(),
        fifo.display().to_string(),
    ]);
    refused_with(&s.dir, a, "is not a regular file", &[]);
    // a missing file and a directory
    for (path, needle) in [
        (
            s.dir.0.join("pki/missing.pem"),
            "cannot read the client intermediates file",
        ),
        (s.dir.0.join("pki"), "is not a regular file"),
    ] {
        let mut a = args(&s, false);
        a.extend([
            "--tls-client-intermediates".into(),
            path.display().to_string(),
        ]);
        refused_with(&s.dir, a, needle, &[]);
    }
    // writable by group or by other
    for mode in [0o664, 0o646, 0o666] {
        rewrite_file(&s.inters, &s.inter.pem, mode);
        refused(&s, "is writable by group or other");
    }
    rewrite_file(&s.inters, &s.inter.pem, 0o644);
    // over 4 MiB (sparse), and a file that grows is cut by the bounded read (the size check is the one tested here)
    let big = s.dir.0.join("pki/big.pem");
    let f = std::fs::File::create(&big).unwrap();
    f.set_len(5 * 1024 * 1024).unwrap();
    std::fs::set_permissions(&big, std::os::unix::fs::PermissionsExt::from_mode(0o644)).unwrap();
    let mut a = args(&s, false);
    a.extend([
        "--tls-client-intermediates".into(),
        big.display().to_string(),
    ]);
    refused_with(&s.dir, a, "larger than 4 MiB: it is not read", &[]);
}

#[test]
fn an_empty_junk_crl_only_or_malformed_intermediates_file_is_refused_in_words() {
    let s = setup();
    let bad_der = pem_of(&[0x30, 0x03, 0x01, 0x01, 0xff]).replace("X509 CRL", "CERTIFICATE");
    let too_many: String = (0..17).map(|_| s.inter.pem.clone()).collect();
    for (text, needle) in [
        ("", "holds no certificate"),
        ("\n  \n", "holds no certificate"),
        ("this is not a certificate\n", "holds no certificate"),
        (
            "-----BEGIN CERTIFICATE-----\n!!!not base64!!!\n-----END CERTIFICATE-----\n",
            "is not valid PEM",
        ),
        // a PEM block of another kind is skipped, so a file of only that holds no certificate
        (&s.inter.crl_pem(&[], false), "holds no certificate"),
        (&bad_der, "whose subject cannot be read"),
        (&too_many, "more than 16 certificates"),
    ] {
        rewrite_file(&s.inters, text, 0o644);
        refused(&s, needle);
    }
    // a good certificate followed by a malformed one refuses the whole file
    rewrite_file(&s.inters, &format!("{}{bad_der}", s.inter.pem), 0o644);
    refused(&s, "whose subject cannot be read");
    // sixteen certificates are accepted
    let sixteen: String = (0..15)
        .map(|n| Ca::new(&format!("other {n}")).pem)
        .chain([s.inter.pem.clone()])
        .collect();
    rewrite_file(&s.inters, &sixteen, 0o644);
    let (_server, log) = started(&s, true);
    assert!(!log.contains("warning:"), "{log}");
}

#[test]
fn the_log_and_the_messages_never_hold_a_subject_a_serial_or_the_file_content() {
    let s = setup();
    let stranger = Ca::new("stranger ca");
    rewrite_file(&s.crl, &stranger.crl_pem(&[4242], false), 0o644);
    refused_with(
        &s.dir,
        args(&s, true),
        "client intermediates file",
        &["canary-inter-7731", "stranger ca", "4242", "BEGIN"],
    );
}

#[test]
fn a_listed_intermediate_is_never_a_trust_anchor() {
    use rcgen::KeyUsagePurpose::{CrlSign, KeyCertSign};
    let s = setup();
    // an intermediate of ANOTHER root, which the server does not trust: its client chains to no trust anchor
    let other_root = Ca::new("other root");
    let other = other_root.intermediate("other intermediate", vec![KeyCertSign, CrlSign]);
    rewrite_file(&s.inters, &other.pem, 0o644);
    rewrite_file(&s.crl, &other.crl_pem(&[], false), 0o644);
    let (server, log) = started(&s, true);
    assert!(!log.contains("warning:"), "{log}");
    let stranger = other.issue("worker", Use::Client);
    let config = client_config(&s.server_ca, Some(&stranger), None);
    get_status(server.addr, &config, "/nope").expect_err(
        "a chain that reaches no trust anchor is refused, whatever the intermediates file lists",
    );
    // a client of a trusted root's intermediate that sends no chain is refused too: the listed certificate is not used
    // to build the path either
    let own = s
        .client_ca
        .intermediate("own intermediate", vec![KeyCertSign, CrlSign]);
    rewrite_file(&s.inters, &format!("{}{}", other.pem, own.pem), 0o644);
    rewrite_file(&s.crl, &own.crl_pem(&[], false), 0o644);
    let leaf = own.issue("worker", Use::Client);
    drop(server);
    let (server, _) = started(&s, true);
    let full = client_config(&s.server_ca, Some(&leaf), None);
    assert_eq!(get_status(server.addr, &full, "/nope").unwrap(), 404);
    let bare = client_config(&s.server_ca, Some(&leaf.without_chain()), None);
    get_status(server.addr, &bare, "/nope").expect_err(
        "the listed intermediate is not used to build a path for a client that omits it",
    );
}

#[test]
fn the_same_intermediate_in_the_ca_file_would_be_an_anchor_which_is_why_the_test_above_means_something()
 {
    use rcgen::KeyUsagePurpose::{CrlSign, KeyCertSign};
    let s = setup();
    let other_root = Ca::new("other root");
    let other = other_root.intermediate("other intermediate", vec![KeyCertSign, CrlSign]);
    // the control: as a trust anchor (the CA file) the same certificate admits its client
    rewrite_file(&s.files.ca, &other.pem, 0o644);
    rewrite_file(&s.crl, &other.crl_pem(&[], false), 0o644);
    let (server, _) = started(&s, false);
    let leaf = other.issue("worker", Use::Client);
    let config = client_config(&s.server_ca, Some(&leaf), None);
    assert_eq!(get_status(server.addr, &config, "/nope").unwrap(), 404);
}

#[test]
fn a_client_of_an_expired_intermediate_is_judged_as_before_with_or_without_the_flag() {
    let s = setup();
    let expired = s
        .client_ca
        .expired_intermediate("expired intermediate", vec![KeyCertSign, CrlSign]);
    rewrite_file(&s.inters, &expired.pem, 0o644);
    rewrite_file(&s.crl, &expired.crl_pem(&[], false), 0o644);
    let leaf = expired.issue("worker", Use::Client);
    let mut seen = Vec::new();
    for flag in [true, false] {
        let (server, _) = started(&s, flag);
        let config = client_config(&s.server_ca, Some(&leaf), None);
        let err = get_status(server.addr, &config, "/nope")
            .expect_err("the chain runs through the root, so the expired intermediate is refused")
            .to_string();
        assert!(err.contains("Expired"), "{err}");
        seen.push(err);
    }
    assert_eq!(
        seen[0], seen[1],
        "listing the intermediate changes no verdict"
    );
}

#[test]
fn with_the_flag_a_crl_of_a_listed_issuer_that_its_certificate_did_not_sign_is_refused_at_start() {
    let s = setup();
    // the issuer NAME matches the listed intermediate, the signature is another key's: at handshake time it would refuse
    // every client of that issuer, so it is a refusal at start (a sanity check of the file, not a trust decision)
    rewrite_file(&s.crl, &s.inter.forged_crl_pem(&[4242]), 0o644);
    refused(
        &s,
        "holds a CRL (number 1) whose signature does not verify against the certificate listed for its issuer in the client intermediates file",
    );
    // a good CRL first and a forged one of the same name second: the second is the one named
    let other = s
        .client_ca
        .intermediate("second-inter", vec![KeyCertSign, CrlSign]);
    let mut both_certs = s.inter.pem.clone();
    both_certs.push_str(&other.pem);
    rewrite_file(&s.inters, &both_certs, 0o644);
    let both = format!(
        "{}{}",
        s.inter.crl_pem(&[], false),
        other.forged_crl_pem(&[])
    );
    rewrite_file(&s.crl, &both, 0o644);
    refused(&s, "(number 2)");
    // without the flag the same file still starts (the check belongs to the strict option)
    let (_server, log) = started(&s, false);
    assert!(!log.contains("signature"), "{log}");
}

#[test]
fn with_the_flag_a_properly_signed_crl_of_a_listed_issuer_still_starts() {
    let s = setup();
    let (_server, log) = started(&s, true);
    assert!(!log.contains("warning:"), "{log}");
}
