//! What a hook digest prints and what it costs a session (#1406): the digest text for the level of the session budget
//! spent, and that level from the bytes the store says were injected.

use super::*;

/// What the rooms have to say to a session that has had `level` of its byte budget spent (#1406). The notes on rooms
/// the guard would not show (a room paused by the admin, one the member may not read) are a standing condition, not
/// news: they are printed at the session start and at a prompt, where the agent decides what to do next, and not at
/// every tool call, where the same fixed lines would be paid for thousands of times. A hook with nothing new prints
/// nothing.
pub(super) fn digest_text(
    d: &klatch::gateway::Digest,
    level: klatch::domain::BudgetLevel,
    prompt: bool,
) -> String {
    let mut text = if prompt {
        refused_text(d).0
    } else {
        String::new()
    };
    text.push_str(&klatch::domain::render_hook_at(&d.rooms, d.now, level));
    text
}

/// How much of its byte budget this member's session has spent: the bytes its hooks injected since its latest
/// `SessionStart`. A store that cannot say counts as none spent.
pub(super) fn session_level(t: &dyn Transport, me: &str) -> klatch::domain::BudgetLevel {
    let used: i64 = t
        .hook_runs(me)
        .map(|runs| runs.iter().map(|r| r.since_start).sum())
        .unwrap_or(0);
    klatch::domain::BudgetLevel::for_used(used.max(0) as u64)
}

#[cfg(test)]
mod digest_cost_tests {
    use super::*;
    use klatch::domain::BudgetLevel;
    use klatch::gateway::{Digest, Refused};

    fn refused_only() -> Digest {
        Digest {
            rooms: Vec::new(),
            refused: vec![Refused {
                room: "vault".into(),
                reason: "clearance".into(),
            }],
            now: 0,
        }
    }

    #[test]
    fn a_refused_room_alone_prints_nothing_at_a_tool_call_at_any_level_but_is_told_at_a_prompt() {
        for level in [
            BudgetLevel::Full,
            BudgetLevel::Short,
            BudgetLevel::Few,
            BudgetLevel::CountOnly,
        ] {
            assert_eq!(digest_text(&refused_only(), level, false), "");
            let said = digest_text(&refused_only(), level, true);
            assert!(said.contains("room vault not shown: clearance"), "{said}");
        }
    }

    #[test]
    fn a_refused_room_is_still_told_at_the_session_start() {
        let (text, n) = refused_text(&refused_only());
        assert!(text.contains("room vault not shown: clearance"), "{text}");
        assert_eq!(n, klatch::domain::printed_len(&text));
    }
}
