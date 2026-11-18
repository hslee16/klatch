//! Boundaries of the tool guard that the main tests leave open (#719): which claims count as path claims, where the
//! text clipping starts, the seconds left in the block message, and the two operands of the Bash condition.

use super::*;

fn held(path: &str, owner: &str, reason: &str) -> Held {
    Held {
        room: "r".into(),
        claim: Claim {
            path: path.into(),
            owner: owner.into(),
            reason: reason.into(),
            expires: 1_100,
            scope: String::new(),
        },
    }
}

fn asker() -> Asker<'static> {
    Asker {
        member: "alice",
        scope: "",
        now: 1_000,
    }
}

fn edit_message(h: Held, target: &str) -> String {
    let t = [target.to_string()];
    let call = ToolCall {
        tool: "Edit",
        targets: &t,
        command: "",
        cwd: "/w",
    };
    match judge_tool(&call, &asker(), &[h]) {
        ToolVerdict::Block(m) => m,
        other => panic!("{other:?}"),
    }
}

fn judge_edit(h: Held, target: &str) -> ToolVerdict {
    let t = [target.to_string()];
    let call = ToolCall {
        tool: "Edit",
        targets: &t,
        command: "",
        cwd: "/w",
    };
    judge_tool(&call, &asker(), &[h])
}

#[test]
fn a_ticket_or_ask_claim_never_blocks_an_edit_of_the_path_it_is_spelled_like() {
    for (claim, target) in [("ask:3", "ask:3"), ("ask:", "ask:"), ("ask:7/x", "ask:7/x")] {
        assert_eq!(
            judge_edit(held(claim, "bob", ""), target),
            ToolVerdict::Allow,
            "{claim}"
        );
    }
}

#[test]
fn a_one_character_claim_is_not_a_path_claim_but_two_characters_are() {
    assert_eq!(judge_edit(held("a", "bob", ""), "a"), ToolVerdict::Allow);
    assert_eq!(judge_edit(held("a", "bob", ""), "/w/a"), ToolVerdict::Allow);
    assert!(matches!(
        judge_edit(held("ab", "bob", ""), "ab"),
        ToolVerdict::Block(_)
    ));
}

#[test]
fn a_claim_on_the_root_blocks_nothing() {
    for target in ["/", "/w/a.rs", "/x"] {
        assert_eq!(
            judge_edit(held("/", "bob", ""), target),
            ToolVerdict::Allow,
            "{target}"
        );
    }
}

#[test]
fn a_shell_command_naming_an_ask_or_one_character_claim_is_not_warned_about() {
    for claim in ["ask:3", "a", "/"] {
        let call = ToolCall {
            tool: "Bash",
            targets: &[],
            command: "echo ask:3 a / /w",
            cwd: "/w",
        };
        assert_eq!(
            judge_tool(&call, &asker(), &[held(claim, "bob", "")]),
            ToolVerdict::Allow,
            "{claim}"
        );
    }
}

#[test]
fn the_seconds_left_are_the_expiry_minus_now() {
    let m = edit_message(held("src/a", "bob", ""), "/w/src/a");
    assert!(m.contains("(100s left)"), "{m}");
    let mut late = held("src/a", "bob", "");
    late.claim.expires = 1_001;
    assert!(edit_message(late, "/w/src/a").contains("(1s left)"));
}

#[test]
fn text_of_exactly_the_limit_is_kept_and_one_more_is_clipped() {
    // limits: path 200, owner 40, room 40, reason 100
    for (len, kept) in [(40, true), (41, false)] {
        let owner = "o".repeat(len);
        let m = edit_message(held("src/a", &owner, ""), "/w/src/a");
        assert_eq!(m.contains(&format!("{owner} in room")), kept, "owner {len}");
        assert_eq!(m.contains("..."), !kept, "owner {len}: {m}");
    }
    for (len, kept) in [(100, true), (101, false)] {
        let reason = "r".repeat(len);
        let m = edit_message(held("src/a", "bob", &reason), "/w/src/a");
        assert_eq!(
            m.contains(&format!("data: {reason})")),
            kept,
            "reason {len}"
        );
        assert_eq!(m.contains("..."), !kept, "reason {len}");
    }
    for (len, kept) in [(200, true), (201, false)] {
        let path = format!("/w/{}", "p".repeat(len - 3));
        let m = edit_message(held(&path, "bob", ""), &path);
        assert_eq!(
            m.contains(&format!("{path} is claimed")),
            kept,
            "path {len}"
        );
        assert_eq!(m.contains("..."), !kept, "path {len}");
    }
}

#[test]
fn a_clipped_text_keeps_exactly_the_limit_in_characters_not_bytes() {
    let owner = "é".repeat(41);
    let m = edit_message(held("src/a", &owner, ""), "/w/src/a");
    assert!(m.contains(&format!("{}... in room", "é".repeat(40))), "{m}");
}

#[test]
fn a_command_is_only_judged_for_the_shell_tool() {
    let c = [held("src/api.rs", "bob", "")];
    for tool in ["Read", "Grep", "Glob"] {
        let call = ToolCall {
            tool,
            targets: &[],
            command: "sed -i s/a/b/ src/api.rs",
            cwd: "/w",
        };
        assert_eq!(
            judge_tool(&call, &asker(), &c),
            ToolVerdict::Allow,
            "{tool}"
        );
    }
    let empty = ToolCall {
        tool: "Bash",
        targets: &[],
        command: "",
        cwd: "/w",
    };
    assert_eq!(judge_tool(&empty, &asker(), &c), ToolVerdict::Allow);
    let named = ToolCall {
        command: "sed -i s/a/b/ src/api.rs",
        ..empty
    };
    assert!(matches!(
        judge_tool(&named, &asker(), &c),
        ToolVerdict::Warn(_)
    ));
}
