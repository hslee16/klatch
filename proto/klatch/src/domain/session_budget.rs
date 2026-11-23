//! The byte budget a session's hooks spend (#1406). A hook prints into the model's context, so what a session has had
//! printed to it is a cost nobody saw. This module holds the two budgets and the one rule that follows from them: as
//! the bytes already injected into a session near its budget, the digest gets shorter, then fewer, then only a count
//! with the exact command to read more. Only sizes are used here, never text.
//!
//! The defaults and the reasons for them:
//!
//! - `PROMPT_BUDGET_BYTES` (4000): what one `UserPromptSubmit` or `PostToolUse` digest may print. A message line is
//!   about 250 bytes and a room header about 270, so 4000 holds a room header and a dozen short messages, which is more
//!   than an agent acts on between two tool calls. The hard cap (`DIGEST_MAX_BYTES`, 8000) exists because Claude Code
//!   keeps only a 2 KB preview past 9.8 KB; the prompt budget sits well under it so one busy turn cannot spend a tenth of
//!   a session.
//! - `SESSION_BUDGET_BYTES` (48000): about 12 000 tokens, a few percent of a 200 000 token window. The convention
//!   (3 to 4 KB) and one backlog at the start take a quarter of it; the rest is about a dozen full digests, and a
//!   quiet session never reaches it because a hook with nothing new prints nothing.
//!
//! "Session" means the bytes injected since the member's latest `SessionStart` (a resumed or compacted session starts
//! its count again, because its context was rebuilt).

use super::budget::{DIGEST_MAX_BYTES, DIGEST_MAX_MESSAGES, DIGEST_MESSAGE_CHARS};

/// What one prompt or tool-call digest may print, escaped, at the normal level.
pub const PROMPT_BUDGET_BYTES: usize = 4000;
/// What a session may have injected before its digests shrink to a count.
pub const SESSION_BUDGET_BYTES: u64 = 48_000;

/// How far into its session budget a session is, and so how much of a digest it is shown. The thresholds are shares of
/// `SESSION_BUDGET_BYTES`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BudgetLevel {
    /// Under half: whole digests (up to the prompt budget).
    Full,
    /// Half to three quarters: shorter lines and a compact room header, at most 8 messages.
    Short,
    /// Three quarters to the budget: 160 character lines, at most 3 messages, a small byte budget.
    Few,
    /// At or past the budget: only addressed messages (direct messages and mentions) in short form, the member's own
    /// asks and claim changes, and for the rest a count with the command that reads it.
    CountOnly,
}

impl BudgetLevel {
    /// The level of a session that has had `used` bytes injected.
    pub fn for_used(used: u64) -> Self {
        match used {
            u if u >= SESSION_BUDGET_BYTES => BudgetLevel::CountOnly,
            u if u >= SESSION_BUDGET_BYTES / 4 * 3 => BudgetLevel::Few,
            u if u >= SESSION_BUDGET_BYTES / 2 => BudgetLevel::Short,
            _ => BudgetLevel::Full,
        }
    }

    /// Characters of one message line.
    pub fn message_chars(self) -> usize {
        match self {
            BudgetLevel::Full => DIGEST_MESSAGE_CHARS,
            BudgetLevel::Short => 300,
            BudgetLevel::Few | BudgetLevel::CountOnly => 160,
        }
    }

    /// Messages one room contributes (addressed ones first; in `CountOnly` only addressed ones are kept).
    pub fn max_messages(self) -> usize {
        match self {
            BudgetLevel::Full => DIGEST_MAX_MESSAGES,
            BudgetLevel::Short => 8,
            BudgetLevel::Few => 3,
            BudgetLevel::CountOnly => DIGEST_MAX_MESSAGES,
        }
    }

    /// The bytes one digest may print at this level.
    pub fn digest_bytes(self) -> usize {
        match self {
            BudgetLevel::Full => PROMPT_BUDGET_BYTES.min(DIGEST_MAX_BYTES),
            BudgetLevel::Short => 2500,
            BudgetLevel::Few => 1500,
            BudgetLevel::CountOnly => 1000,
        }
    }

    /// Whether the room header is the compact one (the full header says the same thing in 270 bytes).
    pub fn compact_header(self) -> bool {
        self != BudgetLevel::Full
    }

    /// Whether only addressed messages are shown and the rest is a count.
    pub fn count_only(self) -> bool {
        self == BudgetLevel::CountOnly
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_level_follows_the_share_of_the_session_budget_spent() {
        assert_eq!(BudgetLevel::for_used(0), BudgetLevel::Full);
        assert_eq!(
            BudgetLevel::for_used(SESSION_BUDGET_BYTES / 2 - 1),
            BudgetLevel::Full
        );
        assert_eq!(
            BudgetLevel::for_used(SESSION_BUDGET_BYTES / 2),
            BudgetLevel::Short
        );
        assert_eq!(
            BudgetLevel::for_used(SESSION_BUDGET_BYTES / 4 * 3),
            BudgetLevel::Few
        );
        assert_eq!(
            BudgetLevel::for_used(SESSION_BUDGET_BYTES - 1),
            BudgetLevel::Few
        );
        assert_eq!(
            BudgetLevel::for_used(SESSION_BUDGET_BYTES),
            BudgetLevel::CountOnly
        );
        assert_eq!(BudgetLevel::for_used(u64::MAX), BudgetLevel::CountOnly);
    }

    #[test]
    fn each_level_shows_no_more_than_the_one_before() {
        let order = [
            BudgetLevel::Full,
            BudgetLevel::Short,
            BudgetLevel::Few,
            BudgetLevel::CountOnly,
        ];
        for pair in order.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            assert!(b.message_chars() <= a.message_chars());
            assert!(b.digest_bytes() <= a.digest_bytes());
        }
        assert!(BudgetLevel::Short.max_messages() < BudgetLevel::Full.max_messages());
        assert!(BudgetLevel::Few.max_messages() < BudgetLevel::Short.max_messages());
    }

    #[test]
    fn the_defaults_are_inside_the_hook_cap_and_the_prompt_budget_is_a_fraction_of_the_session() {
        const { assert!(PROMPT_BUDGET_BYTES <= DIGEST_MAX_BYTES) };
        assert_eq!(PROMPT_BUDGET_BYTES, 4000);
        assert_eq!(SESSION_BUDGET_BYTES, 48_000);
        const { assert!(SESSION_BUDGET_BYTES >= 10 * PROMPT_BUDGET_BYTES as u64) };
        assert_eq!(BudgetLevel::Full.digest_bytes(), PROMPT_BUDGET_BYTES);
    }
}
