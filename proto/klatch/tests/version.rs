//! `klatch --version` says which wire protocol and store schema this binary speaks (#433), so a user or a script can
//! tell, before opening a store, whether this binary can read it.

use std::process::Command;

#[test]
fn the_version_line_names_the_protocol_and_the_store_schema() {
    let out = Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env_remove("KLATCH_IDENTITY_FILE")
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    let want = format!(
        "klatch {} (protocol {}, store schema {}",
        env!("CARGO_PKG_VERSION"),
        klatch::protocol::PROTOCOL_VERSION,
        klatch::store::SCHEMA_VERSION
    );
    assert!(text.starts_with(&want), "wanted '{want}...', got '{text}'");
    assert_eq!(text.trim_end().lines().count(), 1, "one line: {text}");
}
