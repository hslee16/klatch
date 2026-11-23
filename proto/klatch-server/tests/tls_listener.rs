//! `klatch-server https` (#108, epic #13, decision of #157): the klatch wire over HTTP and server-sent events behind a
//! mutually authenticated TLS listener. Covered here: the handshake is the gate (no client certificate, a certificate of
//! another CA, an expired one: refused; a valid one: accepted, and still `401` without a worker credential, because the
//! certificate gates the port and names no member); a plaintext client gets nothing; TLS 1.2 is the floor; the server
//! refuses to start with a key file anyone else can read, without a client CA, with a certificate that does not fit its
//! key, or off loopback unless that was asked for in words; no message of a refusal holds key material. Certificates are
//! throwaway (rcgen, a dev-dependency). The wire itself is compared with the loopback listener in `tls_wire.rs`.

mod tls_common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;
use tls_common::*;

/// A server with a valid set-up: its certificate is from `server_ca`, clients must come from `client_ca`.
struct Setup {
    dir: Dir,
    server_ca: Ca,
    client_ca: Ca,
    server: Server,
}

fn setup() -> Setup {
    let dir = Dir::new();
    let (server_ca, client_ca) = (Ca::new("server ca"), Ca::new("client ca"));
    let files = server_files(
        &dir,
        &server_ca.issue("klatch", Use::Server),
        &client_ca,
        0o600,
    );
    let server = Server::started(&dir, "https", &tls_args(&files));
    Setup {
        dir,
        server_ca,
        client_ca,
        server,
    }
}

#[test]
fn a_valid_client_certificate_is_served_and_still_needs_a_worker_credential() {
    let s = setup();
    let leaf = s.client_ca.issue("worker-1", Use::Client);
    let config = client_config(&s.server_ca, Some(&leaf), None);
    assert_eq!(get_status(s.server.addr, &config, "/nope").unwrap(), 404);
    // the certificate names no member: the route is the credential's, so it is the one 401
    assert_eq!(
        get_status(s.server.addr, &config, "/v1/stream").unwrap(),
        401
    );
    drop(s.dir);
}

#[test]
fn a_client_without_a_certificate_is_refused_at_the_handshake() {
    let s = setup();
    let config = client_config(&s.server_ca, None, None);
    assert!(get_status(s.server.addr, &config, "/v1/stream").is_err());
    drop(s.dir);
}

#[test]
fn a_certificate_from_another_ca_is_refused() {
    let s = setup();
    let stranger = Ca::new("stranger").issue("worker-1", Use::Client);
    let config = client_config(&s.server_ca, Some(&stranger), None);
    assert!(get_status(s.server.addr, &config, "/v1/stream").is_err());
    drop(s.dir);
}

#[test]
fn an_expired_certificate_is_refused() {
    let s = setup();
    let old = s.client_ca.issue("worker-1", Use::ExpiredClient);
    let config = client_config(&s.server_ca, Some(&old), None);
    assert!(get_status(s.server.addr, &config, "/v1/stream").is_err());
    drop(s.dir);
}

#[test]
fn a_server_certificate_used_as_a_client_certificate_is_refused() {
    // the server's own certificate chains to the server CA, not the client CA, and has no client-auth usage
    let s = setup();
    let wrong_use = s.server_ca.issue("klatch", Use::Server);
    let config = client_config(&s.server_ca, Some(&wrong_use), None);
    assert!(get_status(s.server.addr, &config, "/v1/stream").is_err());
    drop(s.dir);
}

#[test]
fn a_plaintext_client_gets_no_http_answer_from_the_tls_port() {
    let s = setup();
    let mut sock = TcpStream::connect(s.server.addr).unwrap();
    sock.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let _ =
        sock.write_all(b"GET /v1/stream HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer x\r\n\r\n");
    let mut got = Vec::new();
    let _ = sock.read_to_end(&mut got);
    let text = String::from_utf8_lossy(&got);
    assert!(
        !text.contains("HTTP/1"),
        "plaintext got an HTTP answer: {text:?}"
    );
    assert!(
        !text.contains("401") && !text.contains("unauthorized"),
        "{text:?}"
    );
    drop(s.dir);
}

#[test]
fn tls_1_2_is_accepted_and_a_client_that_trusts_the_wrong_server_ca_refuses_the_server() {
    let s = setup();
    let leaf = s.client_ca.issue("worker-1", Use::Client);
    let v12 = client_config(&s.server_ca, Some(&leaf), Some(&[&rustls::version::TLS12]));
    assert_eq!(get_status(s.server.addr, &v12, "/nope").unwrap(), 404);
    let v13 = client_config(&s.server_ca, Some(&leaf), Some(&[&rustls::version::TLS13]));
    assert_eq!(get_status(s.server.addr, &v13, "/nope").unwrap(), 404);
    // no skipped verification on the other side either: a client that does not trust the server's CA cannot connect
    let wrong = client_config(&Ca::new("other"), Some(&leaf), None);
    assert!(tls_connect(s.server.addr, &wrong).is_err());
    drop(s.dir);
}

#[test]
fn a_client_that_offers_only_tls_1_1_is_refused() {
    // rustls cannot offer 1.1; the system's openssl can, when it is there (macOS ships LibreSSL, Linux usually OpenSSL)
    let Some(openssl) = ["/usr/bin/openssl", "/opt/homebrew/bin/openssl"]
        .into_iter()
        .find(|p| std::path::Path::new(p).exists())
    else {
        eprintln!("skipped: no openssl to offer TLS 1.1");
        return;
    };
    let s = setup();
    let out = std::process::Command::new(openssl)
        .args([
            "s_client",
            "-tls1_1",
            "-connect",
            &s.server.addr.to_string(),
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    // no cipher was agreed: the server answered with an alert (or the tool cannot speak 1.1, and then nothing connected)
    let refused = [
        "alert",
        "no protocols available",
        "unsupported protocol",
        "wrong version",
    ]
    .iter()
    .any(|w| text.to_ascii_lowercase().contains(w));
    assert!(refused, "{text}");
    assert!(
        !text.contains("Cipher is TLS") && !text.contains("Cipher is ECDHE"),
        "{text}"
    );
    drop(s.dir);
}

// ---------------------------------------------------------------------------------------------------------------
// refusals before anything listens

fn files_for(dir: &Dir, key_mode: u32) -> (Files, Ca) {
    let (server_ca, client_ca) = (Ca::new("server ca"), Ca::new("client ca"));
    let files = server_files(
        dir,
        &server_ca.issue("klatch", Use::Server),
        &client_ca,
        key_mode,
    );
    (files, server_ca)
}

fn assert_refused(dir: &Dir, args: &[String], needle: &str) {
    let (code, err) = refused_start(dir, "https", args);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains(needle), "wanted {needle:?} in: {err}");
    assert!(!err.contains("listening on"), "{err}");
    assert!(
        !err.contains("PRIVATE KEY") && !err.contains("BEGIN"),
        "a message holds key material: {err}"
    );
}

fn listen(addr: &str, files: &Files) -> Vec<String> {
    let mut args = vec!["--listen".to_string(), addr.to_string()];
    args.extend(tls_args(files));
    args
}

#[test]
fn a_key_file_that_group_or_other_can_read_stops_the_server_before_it_listens() {
    for mode in [0o640, 0o604, 0o644, 0o660, 0o666, 0o610] {
        let dir = Dir::new();
        let (files, _) = files_for(&dir, mode);
        assert_refused(&dir, &listen("127.0.0.1:0", &files), "0600");
    }
}

#[test]
fn a_key_file_that_is_a_symlink_or_missing_stops_the_server() {
    let dir = Dir::new();
    let (files, _) = files_for(&dir, 0o600);
    let link = dir.0.join("pki/link.key");
    std::os::unix::fs::symlink(&files.key, &link).unwrap();
    let linked = Files {
        key: link,
        cert: files.cert.clone(),
        ca: files.ca.clone(),
    };
    assert_refused(&dir, &listen("127.0.0.1:0", &linked), "symbolic link");
    let missing = Files {
        key: dir.0.join("pki/none.key"),
        cert: files.cert.clone(),
        ca: files.ca.clone(),
    };
    assert_refused(&dir, &listen("127.0.0.1:0", &missing), "key file");
}

#[test]
fn a_fifo_named_as_the_key_cannot_block_startup_and_is_refused() {
    let dir = Dir::new();
    let (files, _) = files_for(&dir, 0o600);
    let fifo = dir.0.join("pki/fifo.key");
    let path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let fifo_files = Files {
        key: fifo,
        cert: files.cert.clone(),
        ca: files.ca.clone(),
    };
    assert_refused(
        &dir,
        &listen("127.0.0.1:0", &fifo_files),
        "not a regular file",
    );
}

#[test]
fn tls_without_a_client_ca_is_refused() {
    let dir = Dir::new();
    let (files, _) = files_for(&dir, 0o600);
    let mut args = listen("127.0.0.1:0", &files);
    let at = args.iter().position(|a| a == "--tls-client-ca").unwrap();
    args.drain(at..at + 2);
    assert_refused(&dir, &args, "--tls-client-ca");
}

#[test]
fn a_client_ca_file_with_no_certificate_is_refused() {
    let dir = Dir::new();
    let (files, _) = files_for(&dir, 0o600);
    std::fs::write(&files.ca, "not a certificate\n").unwrap();
    // the refusal is the policy's (no root to check a client against), not a side effect of building the verifier
    assert_refused(&dir, &listen("127.0.0.1:0", &files), "without a client CA");
}

#[test]
fn a_certificate_that_does_not_belong_to_the_key_is_refused() {
    let dir = Dir::new();
    let (files, _) = files_for(&dir, 0o600);
    let other = Ca::new("x").issue("other", Use::Server);
    std::fs::write(&files.cert, &other.cert_pem).unwrap();
    assert_refused(&dir, &listen("127.0.0.1:0", &files), "certificate");
}

#[test]
fn an_address_off_loopback_needs_the_explicit_allowance() {
    let dir = Dir::new();
    let (files, _) = files_for(&dir, 0o600);
    for addr in ["0.0.0.0:0", "[::]:0", "192.168.1.5:7443", "[fe80::1]:7443"] {
        assert_refused(&dir, &listen(addr, &files), "--allow-off-loopback");
    }
    // a host name is never resolved, with or without the allowance
    assert_refused(&dir, &listen("localhost:7443", &files), "IP address");
}

#[test]
fn plaintext_http_off_loopback_is_still_refused() {
    let dir = Dir::new();
    for addr in ["0.0.0.0:0", "192.168.1.5:7443"] {
        let args = vec!["--listen".to_string(), addr.to_string()];
        let (code, err) = refused_start(&dir, "http", &args);
        assert_eq!(code, Some(1), "{err}");
        assert!(err.contains("refusing"), "{err}");
    }
    // and the https subcommand has no plaintext mode: no TLS options at all is a usage error
    let args = vec!["--listen".to_string(), "127.0.0.1:0".to_string()];
    let (code, err) = refused_start(&dir, "https", &args);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("--tls-cert"), "{err}");
}

#[test]
fn there_is_no_option_to_skip_verification_or_serve_clients_without_a_certificate() {
    let dir = Dir::new();
    let (files, _) = files_for(&dir, 0o600);
    for flag in [
        "--insecure",
        "--no-client-auth",
        "--tls-optional-client-cert",
        "--skip-verify",
        "--plaintext",
    ] {
        let mut args = listen("127.0.0.1:0", &files);
        args.push(flag.to_string());
        assert_refused(&dir, &args, "unknown option");
    }
}

#[test]
fn the_allowance_off_loopback_starts_the_listener_and_the_log_says_where_and_how() {
    let dir = Dir::new();
    let (files, _) = files_for(&dir, 0o600);
    let mut args = tls_args(&files);
    args.push("--allow-off-loopback".into());
    let server = Server::started(&dir, "https", &args);
    let log = dir.log();
    assert!(log.contains("mutual TLS"), "{log}");
    assert!(!log.contains("PRIVATE KEY"), "{log}");
    drop(server);
}
