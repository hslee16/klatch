//! The binary keeps its store encrypted (epic #152): a key is made on first use and kept apart from the data, a
//! plaintext database from an earlier release is converted, and a store whose key is gone is refused loudly.
mod common;

use common::*;
use std::os::unix::fs::MetadataExt;

const FIXTURE_V2: &[u8] = include_bytes!("fixture-v2.db");

fn bytes_of_everything(dir: &std::path::Path) -> Vec<u8> {
    let mut all = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_file() {
            all.extend(std::fs::read(p).unwrap());
        }
    }
    all
}

fn has(bytes: &[u8], needle: &str) -> bool {
    bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
}

fn key_files(h: &Home) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(h.0.join("keys"))
        .map(|d| d.map(|e| e.unwrap().path()).collect())
        .unwrap_or_default()
}

#[test]
fn a_fresh_store_is_encrypted_with_a_key_made_on_first_use_and_kept_in_its_own_directory() {
    let h = Home::new();
    h.ok("alice", &["join", "vault-room-xyz"]);
    h.ok(
        "alice",
        &["post", "vault-room-xyz", "the-code-is-hunter2-plain"],
    );
    let data = bytes_of_everything(&h.0);
    for secret in ["the-code-is-hunter2-plain", "vault-room-xyz", "alice"] {
        assert!(
            !has(&data, secret),
            "{secret} is readable in the store directory"
        );
    }
    assert!(!data.starts_with(b"SQLite format 3"));
    let keys = key_files(&h);
    assert_eq!(keys.len(), 1, "one key per store: {keys:?}");
    assert_eq!(std::fs::metadata(&keys[0]).unwrap().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(&keys[0]).unwrap().len(), 32);
}

#[test]
fn the_key_never_appears_in_any_output() {
    let h = Home::new();
    let mut seen = String::new();
    for args in [
        vec!["join", "r"],
        vec!["post", "r", "hi"],
        vec!["read", "r"],
        vec!["hook"],
        vec!["who", "r"],
    ] {
        let o = h.run("alice", &args);
        seen.push_str(&String::from_utf8_lossy(&o.stdout));
        seen.push_str(&String::from_utf8_lossy(&o.stderr));
    }
    let key = std::fs::read(&key_files(&h)[0]).unwrap();
    let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    assert!(!seen.contains(&hex) && !seen.to_uppercase().contains(&hex.to_uppercase()));
    assert!(
        !seen.contains("KLATCH-"),
        "a recovery code reached ordinary output"
    );
}

#[test]
fn a_plaintext_database_from_an_earlier_release_is_converted_on_first_use() {
    let h = Home::new();
    std::fs::write(h.0.join("rooms.db"), FIXTURE_V2).unwrap();
    let out = h.ok("carol", &["read", "core", "--peek"]);
    assert!(out.contains("hello from v2"), "{out}");
    let data = bytes_of_everything(&h.0);
    assert!(
        !has(&data, "hello from v2"),
        "the old content is still readable on disk"
    );
    let names: Vec<String> = std::fs::read_dir(&h.0)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names
            .iter()
            .all(|n| !n.ends_with(".old") && !n.ends_with(".encrypting")),
        "{names:?}"
    );
}

#[test]
fn an_encrypted_store_whose_key_is_gone_is_refused_and_no_new_key_is_made() {
    let h = Home::new();
    h.ok("alice", &["post", "r", "kept"]);
    for k in key_files(&h) {
        std::fs::remove_file(k).unwrap();
    }
    let err = h.fail("alice", &["read", "r"]);
    assert!(err.contains("klatch admin recover"), "{err}");
    assert!(
        key_files(&h).is_empty(),
        "a new key was made over an existing store"
    );
}

#[test]
fn a_hook_with_the_key_gone_says_so_in_the_context_and_does_not_fail_the_session() {
    let h = Home::new();
    h.ok("alice", &["post", "r", "kept"]);
    for k in key_files(&h) {
        std::fs::remove_file(k).unwrap();
    }
    let o = h.run("alice", &["hook", "--event", "UserPromptSubmit"]);
    assert!(o.status.success());
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(out.contains("klatch admin recover"), "{out}");
}

#[test]
fn the_key_commands_need_a_real_terminal_like_every_admin_command() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    for args in [
        vec!["admin", "recovery"],
        vec!["admin", "recover", "KLATCH-AAAA"],
        vec!["admin", "rotate-key"],
    ] {
        let err = h.fail("alice", &args);
        assert!(
            err.contains("real terminal") || err.contains("coding-agent session"),
            "{args:?}: {err}"
        );
    }
}
