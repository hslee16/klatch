//! The `asks:` section of the board text.

use super::{BOARD_LIST_LIMIT, Board, Now, more, section};
use crate::domain::flat;
use crate::domain::hook::clip;

/// The `asks:` section.
pub(super) fn render(board: &Board, now: Now, out: &mut String) {
    let none = board.asks.is_empty().then_some("no open asks");
    section(out, "asks:", none);
    for a in board.asks.iter().take(BOARD_LIST_LIMIT) {
        let summary = a.summary(now.0);
        let gap = if summary.is_empty() { "" } else { " " };
        let state = if a.claimed { "claimed" } else { "unclaimed" };
        out.push_str(&format!(
            "  #{} {} {state} {}s{gap}{summary}: {}\n",
            a.seq,
            flat(&a.sender),
            now.since(a.ts),
            clip(&a.text)
        ));
    }
    more(out, board.asks.len(), "asks");
}
