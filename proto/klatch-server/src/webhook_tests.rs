//! Tests of the webhook intake (`webhook.rs`, #1058): the HMAC, the secret and the arguments. The translation of recorded-
//! shape fixtures (NOT recordings; see tests/fixtures/README.md) and the queue are in `webhook_translate_tests.rs`, the
//! request handler without a socket in `webhook_intake_tests.rs`; the fixtures are read here.

use crate::webhook::*;
use serde_json::Value;
use std::path::PathBuf;

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.json"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

pub(crate) fn json(name: &str) -> Value {
    serde_json::from_slice(&fixture(name)).unwrap()
}

pub(crate) const A: &str = "a1b2c3d4e5f6a7b8c9d0a1b2c3d4e5f6a7b8c9d0";
pub(crate) const B: &str = "b2c3d4e5f6a7b8c9d0a1b2c3d4e5f6a7b8c9d0a1";

// ---- HMAC (RFC 4231) and the GitHub signature ----

/// The HMAC-SHA256 of `message` under `key` is `expected` (hex).
fn assert_digest(key: &[u8], message: &[u8], expected: &str) {
    assert_eq!(hex(&signature_digest(key, message)), expected);
}

#[test]
fn signature_digest_matches_rfc_4231() {
    // test case 1
    assert_digest(
        &[0x0b; 20],
        b"Hi There",
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
    );
    // test case 2
    assert_digest(
        b"Jefe",
        b"what do ya want for nothing?",
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
    );
    // test case 3
    assert_digest(
        &[0xaa; 20],
        &[0xdd; 50],
        "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe",
    );
    // test case 6: a key longer than the block size is hashed first
    assert_digest(
        &[0xaa; 131],
        b"Test Using Larger Than Block-Size Key - Hash Key First",
        "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
    );
}

#[test]
fn the_github_documented_signature_verifies() {
    // the example in GitHub's "Validating webhook deliveries"
    let secret = Secret::new(b"It's a Secret to Everybody".to_vec()).unwrap();
    let sig = "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";
    assert!(verify_signature(&secret, b"Hello, World!", Some(sig)));
}

/// A signature header that does not verify `body` under `secret`.
fn assert_rejected(secret: &Secret, body: &[u8], header: Option<&str>) {
    assert!(
        !verify_signature(secret, body, header),
        "verified: {header:?}"
    );
}

#[test]
fn a_missing_wrong_or_malformed_signature_does_not_verify() {
    let secret = Secret::new(b"It's a Secret to Everybody".to_vec()).unwrap();
    let good = "757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";
    let body = b"Hello, World!";
    let mut flipped = good.to_string();
    flipped.replace_range(0..1, "8");
    let bad_headers = [
        None,
        Some(String::new()),
        Some(good.to_string()), // no prefix
        Some(format!("sha1={good}")),
        Some("sha256=".to_string()),
        Some(format!("sha256={}", &good[..62])),
        Some(format!("sha256={good}00")),
        Some(format!("sha256={}zz", &good[..62])),
        // one flipped digit
        Some(format!("sha256={flipped}")),
    ];
    for header in &bad_headers {
        assert_rejected(&secret, body, header.as_deref());
    }
    // a different body, a different secret
    let header = format!("sha256={good}");
    assert_rejected(&secret, b"Hello, World?", Some(&header));
    let other = Secret::new(b"another".to_vec()).unwrap();
    assert_rejected(&other, body, Some(&header));
}

// ---- the secret ----

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("klatch-server-secret-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[cfg(unix)]
fn write_secret(dir: &std::path::Path, text: &str, mode: u32) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join("secret");
    std::fs::write(&p, text).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
    p
}

#[cfg(unix)]
#[test]
fn the_secret_comes_from_a_private_file_and_a_trailing_newline_is_dropped() {
    let dir = temp_dir("file");
    let p = write_secret(&dir, "s3cret-value\n", 0o600);
    let s = Secret::load(Some(&p), None, &|_| None).unwrap();
    assert!(verify_signature(
        &s,
        b"x",
        Some(&format!(
            "sha256={}",
            hex(&signature_digest(b"s3cret-value", b"x"))
        ))
    ));
    // a file others can read is refused
    let open = write_secret(&dir, "s3cret-value\n", 0o644);
    let e = Secret::load(Some(&open), None, &|_| None)
        .unwrap_err()
        .to_string();
    assert!(e.contains("readable"), "{e}");
    assert!(!e.contains("s3cret-value"), "{e}");
    // an empty file is refused
    let empty = write_secret(&dir, "\n", 0o600);
    assert!(Secret::load(Some(&empty), None, &|_| None).is_err());
    // a missing file is refused
    assert!(Secret::load(Some(&dir.join("nope")), None, &|_| None).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_secret_can_come_from_a_named_environment_variable() {
    let lookup = |n: &str| (n == "HOOK_SECRET").then(|| "from-env".to_string());
    let s = Secret::load(None, Some("HOOK_SECRET"), &lookup).unwrap();
    assert!(verify_signature(
        &s,
        b"x",
        Some(&format!(
            "sha256={}",
            hex(&signature_digest(b"from-env", b"x"))
        ))
    ));
    let e = Secret::load(None, Some("UNSET_ONE"), &lookup)
        .unwrap_err()
        .to_string();
    assert!(e.contains("UNSET_ONE"), "{e}");
    assert!(Secret::load(None, Some("EMPTY"), &|_| Some(String::new())).is_err());
}

#[test]
fn no_secret_configuration_is_an_error_and_so_are_two() {
    let e = Secret::load(None, None, &|_| None).unwrap_err().to_string();
    assert!(
        e.contains("--webhook-secret-file") && e.contains("--webhook-secret-env"),
        "{e}"
    );
    let p = PathBuf::from("/nonexistent/secret");
    assert!(Secret::load(Some(&p), Some("X"), &|_| Some("v".into())).is_err());
}

#[test]
fn a_secret_never_shows_in_debug_output() {
    let s = Secret::new(b"hunter2-hunter2".to_vec()).unwrap();
    assert!(!format!("{s:?}").contains("hunter2"));
}

// ---- the arguments ----

fn no_env(_: &str) -> Option<String> {
    None
}

#[test]
fn no_webhook_flags_means_no_intake() {
    assert!(parse_args(&[], &no_env).unwrap().is_none());
}

#[test]
fn a_webhook_without_a_secret_is_a_startup_error() {
    let e = parse_args(&["--webhook", "127.0.0.1:0"], &no_env)
        .err()
        .unwrap()
        .to_string();
    assert!(e.contains("secret"), "{e}");
}

#[test]
fn a_secret_option_without_a_webhook_is_an_error() {
    assert!(parse_args(&["--webhook-secret-env", "X"], &|_| Some("v".into())).is_err());
}

#[test]
fn a_non_loopback_webhook_address_is_refused_naming_the_prerequisites() {
    let env = |_: &str| Some("v".to_string());
    for addr in [
        "0.0.0.0:9000",
        "192.168.0.2:9000",
        "[::]:9000",
        "localhost:9000",
    ] {
        let e = parse_args(&["--webhook", addr, "--webhook-secret-env", "S"], &env)
            .err()
            .unwrap_or_else(|| panic!("{addr} was accepted"))
            .to_string();
        assert!(
            e.contains("refusing") && e.contains("#157") && e.contains("#161"),
            "{addr}: {e}"
        );
    }
}

#[test]
fn good_arguments_give_the_address_and_the_secret() {
    let env = |n: &str| (n == "S").then(|| "v".to_string());
    let a = parse_args(
        &["--webhook", "127.0.0.1:0", "--webhook-secret-env", "S"],
        &env,
    )
    .unwrap()
    .unwrap();
    assert_eq!(a.addr, "127.0.0.1:0");
}

#[test]
fn a_value_that_looks_like_a_secret_is_not_taken_as_an_environment_variable_name() {
    // a person pasting the secret itself after --webhook-secret-env must not have it echoed or used
    let e = parse_args(
        &[
            "--webhook",
            "127.0.0.1:0",
            "--webhook-secret-env",
            "ghp_abc-123=xyz!",
        ],
        &no_env,
    )
    .err()
    .unwrap()
    .to_string();
    assert!(!e.contains("ghp_abc"), "{e}");
}

#[test]
fn unknown_and_valueless_flags_are_errors() {
    assert!(parse_args(&["--webhook"], &no_env).is_err());
    assert!(parse_args(&["--webhook", "127.0.0.1:0", "--secret", "x"], &no_env).is_err());
    assert!(parse_args(&["--webhook-secret"], &no_env).is_err());
}
