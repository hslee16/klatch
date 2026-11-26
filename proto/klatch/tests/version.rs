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
    // The build commit sits between the version and the protocol (#1651); its format is pinned in version_commit.rs.
    let head = format!("klatch {} (commit ", env!("CARGO_PKG_VERSION"));
    let tail = format!(
        ", protocol {}, store schema {}",
        klatch::protocol::PROTOCOL_VERSION,
        klatch::store::SCHEMA_VERSION
    );
    assert!(text.starts_with(&head), "wanted '{head}...', got '{text}'");
    let after = &text[head.len()..];
    let commit_end = after
        .find(&tail)
        .unwrap_or_else(|| panic!("wanted '...{tail}', got '{text}'"));
    assert!(
        !after[..commit_end].is_empty() && !after[..commit_end].contains(' '),
        "commit field: {text}"
    );
    assert_eq!(text.trim_end().lines().count(), 1, "one line: {text}");
}
