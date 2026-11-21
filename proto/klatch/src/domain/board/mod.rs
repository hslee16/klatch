//! Offers and member status, derived from the message log like asks (DESIGN section 25.5). An offer is a
//! message starting `offering:`; it stays current until its sender withdraws it (`withdrawn:`), says
//! `status: done`, or it is older than the backstop lifetime. The gateway also drops an offer whose sender has
//! not been seen for a while (DESIGN section 25.8).

mod asks;
mod members;
mod offers;
mod prs;
mod queue;
mod tasks;

use crate::domain::{OpenAsk, flat};
pub use members::{
    BoardMember, MEMBER_ACTIVE_SECS, MEMBER_RETENTION_SECS, MemberStatus, STATUS_PREFIX,
    ask_reach_note, nudge_skipped_note, render_who, short_dir_name, statuses,
};
pub use offers::{OFFER_PREFIX, OFFER_TTL_SECS, Offer, WITHDRAW_PREFIX, current_offers, is_offer};

/// The most entries the board lists per section; the rest are counted.
pub const BOARD_LIST_LIMIT: usize = 20;
/// The reader's clock: seconds since the epoch. A type of its own so the age helpers cannot be handed some other
/// number, and so an age is never negative when the reader's clock is behind the writer's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Now(i64);

impl Now {
    /// Whole seconds since `then`, never negative.
    fn since(self, then: i64) -> i64 {
        crate::domain::secs_between(self.0, then)
    }
}

/// The current state of a room derived from its log: open asks, current offers and members.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Board {
    pub room: String,
    pub asks: Vec<OpenAsk>,
    pub offers: Vec<Offer>,
    pub members: Vec<BoardMember>,
    /// Members not seen for over `MEMBER_ACTIVE_SECS`, counted instead of listed.
    pub stale_members: usize,
    /// Open durable tasks (working, checkpoint, handoff), with owners that are gone marked orphaned.
    pub tasks: Vec<crate::domain::Task>,
    /// Pull requests said to be open (item 84), with who holds each one's `pr:N` claim, if anyone.
    pub prs: Vec<crate::domain::OpenPr>,
    /// The merge queue (#864): PRs said to be ready and how the integrator says each one is going.
    pub queue: Vec<crate::domain::QueueEntry>,
    /// The member holding the `merge:main` claim now, if any.
    pub merge_holder: Option<String>,
    /// The integrator's halt (#1094), while no later integrator word says it is running again.
    pub halt: Option<crate::domain::Halt>,
}

/// A section heading, with a line naming the section when it is empty so a reader can see there is nothing.
fn section(out: &mut String, heading: &str, empty: Option<&str>) {
    out.push_str(heading);
    out.push('\n');
    if let Some(none) = empty {
        out.push_str(&format!("  {none}\n"));
    }
}

/// A section that exists only when it has lines, bounded like the others.
fn render_list(out: &mut String, heading: &str, lines: &[String], what: &str) {
    if lines.is_empty() {
        return;
    }
    section(out, heading, None);
    for line in lines.iter().take(BOARD_LIST_LIMIT) {
        out.push_str(&format!("  {line}\n"));
    }
    more(out, lines.len(), what);
}

/// How one section of the board is drawn into the text.
type Render = fn(&Board, Now, &mut String);

/// The sections of the board text, in the order they print. To add a section: put its renderer in a new file next to
/// this one (a `pub(super) fn render(board, now, out)`), declare the module above, and add one line here. A section
/// that needs data the `Board` does not carry also adds a field to `Board` and one line where the gateway fills it
/// (`gateway/gateway/board/mod.rs`).
const SECTIONS: &[Render] = &[
    asks::render,
    offers::render,
    tasks::render,
    prs::render,
    queue::render,
    members::render,
];

/// The board as text for a person or an agent: one bounded line per entry, with ask and offer text treated
/// as data. Empty sections are named so a reader can see there is nothing.
pub fn render_board(board: &Board, now: i64) -> String {
    let now = Now(now);
    let mut out = format!("[board {}]\n", flat(&board.room));
    for render in SECTIONS {
        render(board, now, &mut out);
    }
    out
}

/// A line saying how many entries of a section were not listed.
fn more(out: &mut String, total: usize, what: &str) {
    if total > BOARD_LIST_LIMIT {
        out.push_str(&format!("  and {} more {what}\n", total - BOARD_LIST_LIMIT));
    }
}

/// The prefixes of every word that `statuses` and `current_offers` fold, for a store query that reads only those
/// messages at any depth (#731).
pub const BOARD_WORDS: [&str; 3] = [OFFER_PREFIX, WITHDRAW_PREFIX, STATUS_PREFIX];

#[cfg(test)]
mod tests;
