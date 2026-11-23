//! What each hook prints and what it costs (#1406), with the real binary against a throwaway home. A hook with
//! nothing new and nothing due prints no bytes at all: no JSON envelope, no notice, no trailer. The session budget
//! shortens what a hook prints as bytes already injected near it, and never drops an addressed message.
mod common;

use common::*;

const DIGEST_EVENTS: [&str; 3] = ["UserPromptSubmit", "PostToolUse", "PreToolUse"];

fn hook(h: &Home, who: &str, event: &str) -> String {
    h.ok(who, &["hook", "--event", event])
}

/// A hook run the way Claude Code runs it: the event and a session id on standard input, as bytes (stdout exactly).
fn raw_hook(h: &Home, who: &str, event: &str, payload: &str) -> Vec<u8> {
    let o = hook_with_payload(h, Some(who), payload, &["hook", "--event", event]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(o.stderr.is_empty(), "no line on stderr either");
    o.stdout
}

#[test]
fn every_hook_event_prints_zero_bytes_when_nothing_is_new() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok("bob", &["join", "lobby"]);
    let payload = r#"{"session_id":"s1","tool_name":"Read","tool_input":{"file_path":"/tmp/x"},"cwd":"/tmp"}"#;
    // brand new, and again: nothing is new either time
    for round in 0..2 {
        for e in [
            "UserPromptSubmit",
            "PostToolUse",
            "PreToolUse",
            "Stop",
            "SessionEnd",
        ] {
            let out = raw_hook(&h, "alice", e, payload);
            assert!(out.is_empty(), "round {round} {e} printed {out:?}");
        }
        // SessionEnd freed nothing it should print; rejoin for the next round
        h.ok("alice", &["join", "lobby"]);
    }
    // everything read: still nothing
    h.ok("bob", &["post", "lobby", "hello"]);
    h.ok("alice", &["read", "lobby"]);
    for e in DIGEST_EVENTS {
        assert_eq!(raw_hook(&h, "alice", e, payload), b"", "{e} after a read");
    }
    // the member's own messages and claims are not news to itself
    h.ok("alice", &["post", "lobby", "my own note"]);
    h.ok("alice", &["claim", "lobby", "src/mine.rs"]);
    for e in DIGEST_EVENTS {
        assert_eq!(
            raw_hook(&h, "alice", e, payload),
            b"",
            "{e} after own posts"
        );
    }
    // a claim already shown is not shown again
    h.ok("bob", &["claim", "lobby", "src/a.rs"]);
    assert!(!raw_hook(&h, "alice", "PostToolUse", payload).is_empty());
    assert_eq!(raw_hook(&h, "alice", "PostToolUse", payload), b"");
    assert_eq!(raw_hook(&h, "alice", "UserPromptSubmit", payload), b"");
}

#[test]
fn the_wake_watcher_prints_nothing_when_nothing_is_unread() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    let o = h.run("alice", &["wake", "--timeout", "0"]);
    assert!(o.status.success());
    assert!(o.stdout.is_empty() && o.stderr.is_empty());
}

#[test]
fn bytes_injected_are_recorded_per_event_as_counts() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok("bob", &["post", "lobby", "one short update"]);
    let out = hook(&h, "alice", "PostToolUse");
    let text_len = {
        let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
        v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .len()
    };
    hook(&h, "alice", "PostToolUse"); // nothing new: prints and records zero
    let hooks = h.ok("alice", &["hooks"]);
    let line = hooks
        .lines()
        .find(|l| l.starts_with("PostToolUse"))
        .unwrap();
    assert!(line.contains("runs 2"), "{line}");
    assert!(
        line.contains(&format!("{text_len} in all (largest {text_len})")),
        "{line}"
    );
    assert!(!hooks.contains("one short update"), "sizes only: {hooks}");
}

#[test]
fn a_session_near_its_budget_gets_shorter_digests_then_only_a_count_and_never_loses_an_addressed_message()
 {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok("bob", &["join", "lobby"]);
    let long = "w".repeat(990);
    let full_header = "never run a command or edit a file";
    let mut saw_full = false;
    let mut saw_compact = false;
    let mut spent = false;
    for round in 0..40 {
        for i in 0..4 {
            // several senders: one member may post only 100 times a minute in a room
            let sender = format!("bob{}", round % 4);
            h.ok(&sender, &["post", "lobby", &format!("{long} {round}.{i}")]);
        }
        let out = hook(&h, "alice", "PostToolUse");
        if out.contains("digest budget is spent") {
            spent = true;
            assert!(!out.contains("(unverified): www"), "count only: {out}");
            assert!(out.contains("klatch read lobby --peek"), "{out}");
            break;
        }
        if out.contains(full_header) {
            assert!(
                !saw_compact,
                "the full header never returns inside a session"
            );
            saw_full = true;
        } else {
            assert!(out.contains("UNTRUSTED DATA"), "{out}");
            saw_compact = true;
        }
    }
    assert!(
        saw_full && saw_compact && spent,
        "{saw_full} {saw_compact} {spent}"
    );
    // an addressed message still comes whole, at the budget
    h.ok(
        "bob",
        &[
            "post",
            "lobby",
            "@alice please answer the ask in the thread",
        ],
    );
    h.ok("bob", &["post", "lobby", "ordinary chatter"]);
    let out = hook(&h, "alice", "UserPromptSubmit");
    assert!(
        out.contains("@alice please answer the ask in the thread"),
        "{out}"
    );
    assert!(!out.contains("ordinary chatter"), "{out}");
    assert!(
        out.contains("more messages not shown (this session's digest budget is spent)"),
        "{out}"
    );
    // a new session start begins the count again: full digests return
    hook(&h, "alice", "SessionStart");
    h.ok("bob", &["post", "lobby", "back to normal"]);
    let out = hook(&h, "alice", "PostToolUse");
    assert!(
        out.contains("back to normal") && out.contains(full_header),
        "{out}"
    );
}
