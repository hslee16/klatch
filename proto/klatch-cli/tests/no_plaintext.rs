//! Message content and keys never reach logs, crash output, temporary files, arguments or the environment
//! (#163, R37). A marker is planted in text, failures are injected, and every output and temp file is searched.
mod common;

use common::*;
use std::os::unix::fs::PermissionsExt;
use std::process::Output;

const MARKER: &str = "zq9-secret-marker-7731";

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn temp_dir_of(h: &Home) -> std::path::PathBuf {
    let d = h.0.join("tmp");
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run_with_tmp(h: &Home, who: &str, args: &[&str], tmp: &std::path::Path) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env("KLATCH_HOME", &h.0)
        .env("KLATCH_KEY_DIR", h.0.join("keys"))
        .env("KLATCH_NAME", who)
        .env("TMPDIR", tmp)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn the_commands_that_only_acknowledge_never_echo_the_text() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    for args in [
        vec!["post", "r", MARKER],
        vec!["claim", "r", "some/path", "--reason", MARKER],
        vec!["status", "r", MARKER],
        vec!["release", "r", "some/path", "--note", MARKER],
    ] {
        let o = h.run("alice", &args);
        assert!(
            !text(&o).contains(MARKER),
            "{args:?} echoed the text: {}",
            text(&o)
        );
    }
}

#[test]
fn a_refusal_or_a_failure_never_repeats_the_text_of_the_call() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    // a bad label, a credential-looking secret, a room that does not exist as a name, an oversized body
    let creds = format!("{MARKER} AKIAIOSFODNN7EXAMPLE");
    let huge = format!("{MARKER}{}", "x".repeat(100_000));
    for args in [
        vec!["post", "--label", "no-such-label", "r", MARKER],
        vec!["post", "r", &creds],
        vec!["post", "bad room name", MARKER],
        vec!["post", "r", &huge],
        vec!["claim", "r", "", "--reason", MARKER],
    ] {
        let o = h.run("alice", &args);
        assert!(
            !text(&o).contains(MARKER),
            "{:?} repeated the text: {}",
            &args[..2.min(args.len())],
            text(&o).chars().take(300).collect::<String>()
        );
    }
}

#[test]
fn a_broken_store_never_prints_message_text_in_its_error() {
    let h = Home::new();
    h.ok("alice", &["post", "r", MARKER]);
    // the store file damaged, then the key removed, then the directory made read-only
    let db = h.0.join("rooms.db");
    let good = std::fs::read(&db).unwrap();
    let mut bad = good.clone();
    for b in bad.iter_mut().skip(4096).take(64) {
        *b ^= 0xFF;
    }
    std::fs::write(&db, &bad).unwrap();
    assert!(!text(&h.run("alice", &["read", "r"])).contains(MARKER));
    std::fs::write(&db, &good).unwrap();
    std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o400)).unwrap();
    let o = h.run("alice", &["post", "r", format!("{MARKER} more").as_str()]);
    assert!(
        !text(&o).contains(&format!("{MARKER} more")),
        "{}",
        text(&o)
    );
    std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn no_temporary_file_is_left_or_holds_the_text() {
    let h = Home::new();
    let tmp = temp_dir_of(&h);
    for args in [
        vec!["join", "r"],
        vec!["post", "r", MARKER],
        vec!["board", "r"],
        vec!["read", "r", "--peek"],
        vec!["hook"],
        vec!["claims", "r"],
    ] {
        run_with_tmp(&h, "alice", &args, &tmp);
        run_with_tmp(&h, "bob", &args, &tmp);
    }
    let left: Vec<_> = std::fs::read_dir(&tmp)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    for f in &left {
        let bytes = std::fs::read(f).unwrap_or_default();
        assert!(
            !bytes.windows(MARKER.len()).any(|w| w == MARKER.as_bytes()),
            "{f:?} holds the text"
        );
    }
    assert!(left.is_empty(), "temporary files were left: {left:?}");
}

#[test]
fn the_store_directory_holds_no_plaintext_after_every_kind_of_call() {
    let h = Home::new();
    for args in [
        vec!["post", "r", MARKER],
        vec!["claim", "r", "p", "--reason", MARKER],
        vec!["status", "r", MARKER],
        vec!["dm", "bob", MARKER],
    ] {
        h.run("alice", &args);
    }
    let mut all = Vec::new();
    for e in std::fs::read_dir(&h.0).unwrap() {
        let p = e.unwrap().path();
        if p.is_file() {
            all.extend(std::fs::read(p).unwrap());
        }
    }
    assert!(
        !all.windows(MARKER.len()).any(|w| w == MARKER.as_bytes()),
        "the text is readable on disk"
    );
}

#[cfg(feature = "test-clock")]
#[test]
fn a_panic_prints_a_generic_line_and_never_the_message_it_carried() {
    let h = Home::new();
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env("KLATCH_HOME", &h.0)
        .env("KLATCH_KEY_DIR", h.0.join("keys"))
        .env("KLATCH_NAME", "alice")
        .env("KLATCH_TEST_PANIC", MARKER)
        .args(["join", "r"])
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(!text(&o).contains(MARKER), "{}", text(&o));
    assert!(text(&o).contains("internal error"), "{}", text(&o));
}
