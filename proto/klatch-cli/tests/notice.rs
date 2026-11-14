//! End-to-end tests: run the real binary against a throwaway KLATCH_HOME.
mod common;

use common::*;

#[test]
fn session_start_tells_the_human_which_room_and_who_the_session_is() {
    let h = Home::new();
    h.ok("old", &["join", "lobby"]);
    h.ok("old", &["post", "lobby", "seeking: a rare thing"]);
    let out = h.ok("fresh", &["hook", "--event", "SessionStart"]);
    let notice = notice_of(&out).unwrap_or_else(|| panic!("no systemMessage in: {out}"));
    assert!(
        notice.contains("klatch") && notice.contains("joined lobby as fresh"),
        "{notice}"
    );
    assert!(
        notice.contains("2 members") && notice.contains("1 open ask"),
        "{notice}"
    );
    let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    assert!(
        v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("[board lobby]")
    );
}

#[test]
fn a_returning_session_is_told_it_is_back_and_other_events_say_nothing_visible() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    let back = notice_of(&h.ok("alice", &["hook", "--event", "SessionStart"])).expect("a notice");
    assert!(back.contains("back in lobby as alice"), "{back}");
    h.ok("bob", &["post", "lobby", "hi"]);
    assert_eq!(
        notice_of(&h.ok("alice", &["hook", "--event", "UserPromptSubmit"])),
        None
    );
    assert_eq!(
        notice_of(&h.ok("alice", &["hook", "--event", "PostToolUse"])),
        None
    );
}

#[test]
fn a_session_that_stays_out_gets_no_join_notice() {
    let h = Home::new();
    let out = h.ok("quiet", &["hook", "--event", "SessionStart", "--no-join"]);
    assert_eq!(notice_of(&out), None, "{out}");
}

#[test]
fn who_all_lists_every_member_and_a_plain_who_lists_the_recent_ones() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.ok("bob", &["join", "r"]);
    let plain = h.ok("alice", &["who", "r"]);
    assert!(
        plain.contains("alice") && plain.contains("bob"),
        "recent members are listed: {plain}"
    );
    let all = h.ok("alice", &["who", "r", "--all"]);
    assert!(all.contains("alice") && all.contains("bob"), "{all}");
    assert!(!plain.contains("not seen for over"), "{plain}");
}
