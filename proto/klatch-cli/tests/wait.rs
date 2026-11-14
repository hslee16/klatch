//! End-to-end tests: run the real binary against a throwaway KLATCH_HOME.
mod common;

use common::*;

#[test]
fn wait_returns_at_once_when_something_is_unread_and_marks_it_read() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.ok("bob", &["post", "r", "the answer"]);
    let start = std::time::Instant::now();
    // A long timeout and a generous bound: the point is that it did not wait for the timeout, and a loaded CI
    // runner can be slow.
    let out = h.ok("alice", &["wait", "r", "--timeout", "120"]);
    assert!(
        start.elapsed().as_secs() < 60,
        "waited although a message was unread"
    );
    assert!(out.contains("the answer"), "got: {out}");
    assert!(
        !h.ok("alice", &["read", "r"]).contains("the answer"),
        "wait leaves the message unread"
    );
}

#[test]
fn wait_times_out_and_says_so() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    let start = std::time::Instant::now();
    let out = h.ok("alice", &["wait", "r", "--timeout", "1"]);
    let took = start.elapsed();
    assert!(took.as_millis() >= 900, "returned early: {took:?}");
    assert!(took.as_secs() < 30, "overran: {took:?}");
    assert!(
        out.contains("[klatch wait: timeout room=r after=1s]"),
        "got: {out}"
    );
}

#[test]
fn wait_with_zero_timeout_checks_once() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    assert!(
        h.ok("alice", &["wait", "r", "--timeout", "0"])
            .contains("[klatch wait: timeout room=r after=0s]")
    );
    h.ok("bob", &["post", "r", "hi"]);
    assert!(
        h.ok("alice", &["wait", "r", "--timeout", "0"])
            .contains("hi")
    );
}

#[test]
fn wait_wakes_when_another_member_posts() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env("KLATCH_HOME", &h.0)
        .env("KLATCH_KEY_DIR", h.0.join("keys"))
        .env("KLATCH_NAME", "alice")
        .args(["wait", "r", "--timeout", "120"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let start = std::time::Instant::now();
    std::thread::sleep(std::time::Duration::from_millis(700));
    h.ok("bob", &["post", "r", "wake up"]);
    let out = child.wait_with_output().unwrap();
    assert!(start.elapsed().as_secs() < 60, "did not wake promptly");
    assert!(String::from_utf8_lossy(&out.stdout).contains("wake up"));
}

#[test]
fn wait_is_not_woken_by_the_members_own_posts() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.ok("alice", &["post", "r", "mine"]);
    let out = h.ok("alice", &["wait", "r", "--timeout", "1"]);
    assert!(
        out.contains("[klatch wait: timeout room=r after=1s]"),
        "got: {out}"
    );
}

#[test]
fn wait_only_ignores_other_messages_but_leaves_them_unread() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.ok("bob", &["post", "r", "chatter"]);
    let out = h.ok(
        "alice",
        &["wait", "r", "--timeout", "1", "--only", "seeking:"],
    );
    assert!(
        out.contains("[klatch wait: timeout room=r after=1s]"),
        "got: {out}"
    );
    assert!(!out.contains("chatter"));
    assert!(
        h.ok("alice", &["read", "r"]).contains("chatter"),
        "a filter lost a message"
    );
}

#[test]
fn wait_only_returns_a_match_leaves_it_unread_and_does_not_wake_again_for_it() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.ok("bob", &["post", "r", "chatter"]);
    h.ok("bob", &["post", "r", "seeking: a lib"]);
    let out = h.ok(
        "alice",
        &["wait", "r", "--timeout", "5", "--only", "seeking:"],
    );
    assert!(
        out.contains("seeking: a lib") && !out.contains("chatter"),
        "got: {out}"
    );
    let again = h.ok(
        "alice",
        &["wait", "r", "--timeout", "1", "--only", "seeking:"],
    );
    assert!(
        again.contains("[klatch wait: timeout"),
        "woke again for the same message: {again}"
    );
    let read = h.ok("alice", &["read", "r"]);
    assert!(read.contains("chatter") && read.contains("seeking: a lib"));
}

#[test]
fn wait_from_matches_only_that_sender() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.ok("carol", &["post", "r", "noise"]);
    h.ok("bob", &["post", "r", "from bob"]);
    let out = h.ok("alice", &["wait", "r", "--timeout", "5", "--from", "bob"]);
    assert!(
        out.contains("from bob") && !out.contains("noise"),
        "got: {out}"
    );
}

#[test]
fn wait_names_the_room_and_wakes_on_the_first_of_several() {
    let h = Home::new();
    h.ok("alice", &["join", "a"]);
    h.ok("alice", &["join", "b"]);
    h.ok("bob", &["post", "b", "in b"]);
    let out = h.ok("alice", &["wait", "a", "b", "--timeout", "5"]);
    assert!(out.contains("[b ") && out.contains("in b"), "got: {out}");
    assert!(!out.contains("[a "));
    let t = h.ok("alice", &["wait", "a", "b", "--timeout", "1"]);
    assert!(
        t.contains("[klatch wait: timeout room=a,b after=1s]"),
        "got: {t}"
    );
}

#[test]
fn wait_peek_returns_the_message_without_marking_it_read() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.ok("bob", &["post", "r", "keep me"]);
    assert!(
        h.ok("alice", &["wait", "r", "--timeout", "5", "--peek"])
            .contains("keep me")
    );
    assert!(
        h.ok("alice", &["wait", "r", "--timeout", "1", "--peek"])
            .contains("[klatch wait: timeout"),
        "peek wait woke again for the same message"
    );
    assert!(h.ok("alice", &["read", "r"]).contains("keep me"));
}

#[test]
fn a_filtered_or_peeking_wait_keeps_the_waiter_present_in_who() {
    // Compare readings of "last seen" before and after the wait, never an absolute number of seconds: on a loaded
    // CI runner a command can take longer than any fixed bound (the first version failed there).
    fn seen_ago(h: &Home) -> i64 {
        let who = h.ok("bob", &["who", "r"]);
        let line = who.lines().find(|l| l.starts_with("alice")).unwrap();
        line.split("last seen ")
            .nth(1)
            .and_then(|r| r.split('s').next())
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("no last seen in: {line}"))
    }
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    for args in [
        &["wait", "r", "--timeout", "0", "--only", "zzz"][..],
        &["wait", "r", "--timeout", "0", "--peek"][..],
    ] {
        std::thread::sleep(std::time::Duration::from_millis(3200));
        let before = seen_ago(&h);
        assert!(
            before >= 3,
            "the waiter should look idle before the wait: {before}s"
        );
        h.ok("alice", args);
        let after = seen_ago(&h);
        assert!(
            after < before,
            "{args:?} left the waiter looking absent: {before}s before, {after}s after"
        );
    }
}
