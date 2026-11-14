//! End-to-end tests: run the real binary against a throwaway KLATCH_HOME.
mod common;

use common::*;
use klatch::protocol::{ServerFrame, decode_server_line};
use std::io::Write as _;
use std::process::Command;

#[test]
fn posting_nothing_or_only_whitespace_is_refused() {
    let h = Home::new();
    assert!(h.fail("alice", &["post", "r"]).contains("empty"));
    assert!(
        h.fail("alice", &["post", "r", "  ", "\t"])
            .contains("empty")
    );
    assert!(
        h.ok("alice", &["read", "r"]).is_empty(),
        "nothing was stored"
    );
}

#[test]
fn message_ids_are_distinct_across_many_processes_posting_at_once() {
    // Ids combine a timestamp, per-process random bits and a counter. Uniqueness inside one process is
    // unit tested; this checks it across processes that all start and post at the same moment.
    let h = Home::new();
    let mut kids = Vec::new();
    for i in 0..16 {
        let mut child = h.spawn(&format!("w{i}"), &["serve", "--stdio"]);
        {
            let stdin = child.stdin.as_mut().unwrap();
            for j in 0..10 {
                writeln!(
                    stdin,
                    "{}",
                    request(
                        &format!("p{j}"),
                        "post",
                        &format!(r#"{{"room":"r","body":"x{j}"}}"#)
                    )
                )
                .unwrap();
            }
        }
        drop(child.stdin.take());
        kids.push(child);
    }
    let mut ids = std::collections::HashSet::new();
    for k in kids {
        let out = k.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        for line in String::from_utf8(out.stdout).unwrap().lines() {
            if let ServerFrame::Response {
                ok: true,
                result: Some(r),
                ..
            } = decode_server_line(line).unwrap()
            {
                let id = r["id"].as_str().unwrap().to_string();
                assert_eq!(id.len(), 26);
                assert!(ids.insert(id.clone()), "duplicate id {id}");
            }
        }
    }
    assert_eq!(ids.len(), 160);
}

#[test]
fn the_as_member_flag_beats_the_environment_variable() {
    let h = Home::new();
    let out = h.ok("envname", &["--as-member", "flagname", "join", "r"]);
    assert_eq!(out.trim(), "flagname joined r");
}

#[test]
fn an_empty_environment_name_is_fine_when_the_flag_supplies_one() {
    let h = Home::new();
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env("KLATCH_HOME", &h.0)
        .env("KLATCH_KEY_DIR", h.0.join("keys"))
        .env("KLATCH_NAME", "")
        .args(["--as-member", "carol", "join", "r"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

#[test]
fn the_caller_from_the_environment_is_never_verified_in_stored_messages() {
    let h = Home::new();
    h.ok("root", &["post", "r", "I am root"]);
    let f = h.stream("alice", &["r"]);
    let m = f
        .iter()
        .find(|x| matches!(x, ServerFrame::Message { .. }))
        .unwrap();
    assert!(
        matches!(m, ServerFrame::Message { attribution: klatch::protocol::Attribution::Unattributed, sender, .. } if sender == "root")
    );
}

#[test]
fn discovery_that_finds_nothing_says_so_and_asks_for_a_name_without_printing_the_environment() {
    let home = Home::new();
    let o = without_a_name(&home, &home.0, &["--discover", "who", "r"]);
    assert!(!o.status.success());
    let (out, err) = (
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr),
    );
    assert!(out.is_empty(), "{out}");
    assert!(err.contains("no running session"), "{err}");
    assert!(err.contains("KLATCH_NAME"), "{err}");
    for text in [&out, &err] {
        assert!(
            !text.contains("TOKEN-SENTINEL"),
            "environment leaked: {text}"
        );
    }
}

#[test]
fn discovery_is_off_by_default_and_the_registry_is_not_read() {
    let home = Home::new();
    // a registry that would be an error to read: a directory where the record file should be
    let reg = home.0.join(".claude").join("sessions");
    std::fs::create_dir_all(reg.join(format!("{}.json", std::process::id()))).unwrap();
    let o = without_a_name(&home, &home.0, &["who", "r"]);
    assert!(!o.status.success());
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(!err.contains("no running session"), "discovery ran: {err}");
    assert!(err.contains("KLATCH_NAME"), "{err}");
}

#[test]
fn the_environment_switch_turns_discovery_on() {
    let home = Home::new();
    let o = Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env("KLATCH_HOME", &home.0)
        .env("KLATCH_KEY_DIR", home.0.join("keys"))
        .env_remove("KLATCH_NAME")
        .env("KLATCH_DISCOVER", "1")
        .env("HOME", &home.0)
        .args(["who", "r"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&o.stderr).contains("no running session"));
}

#[test]
fn a_typed_name_beats_discovery() {
    let home = Home::new();
    let o = home.run("alice", &["--discover", "join", "r"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(home.ok("alice", &["who", "r"]).contains("alice"));
}

#[test]
fn what_read_prints_marks_a_sender_that_was_not_proven() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok("bob", &["post", "lobby", "typed name only"]);
    let read = h.ok("alice", &["read", "lobby", "--peek"]);
    assert!(read.contains("bob (unverified)] typed name only"), "{read}");
}

#[test]
fn what_a_hook_prints_marks_a_sender_that_was_not_proven() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok("bob", &["post", "lobby", "typed name only"]);
    let hook = h.ok("alice", &["hook"]);
    assert!(
        hook.contains("from bob (unverified): typed name only"),
        "{hook}"
    );
}
