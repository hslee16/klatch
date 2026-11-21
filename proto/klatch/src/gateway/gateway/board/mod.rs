//! What a member is shown unprompted or on request: the board, asks, receipts and the hook digest.
//!
//! One file per section of the board, each a method of the gateway that reads the store for that section only:
//! `members` (the roster), `tasks`, `prs` (open pull requests), `queue` (the merge queue), `asks`, `offers`. The
//! hook digest and receipts have their own files. To add a section of the board: write its `Gateway` method in a new
//! file here, declare the module below, add the field to `domain::Board` (and a renderer, see `domain/board/mod.rs`),
//! and add one line to the `Board` literal in `board` that calls the method.

use super::*;

mod asks;
mod could_answer;
mod digest;
mod members;
mod offers;
mod prs;
mod queue;
mod receipt;
mod tasks;

/// What a narrowed view held back: the count and the newest seq, `None` when nothing (#673).
type Hidden = Option<(usize, i64)>;

impl<S: Store> Gateway<S> {
    /// The room's board: open asks, current offers and members with their last announced status.
    pub fn board(&self, who: &Principal, room: &str) -> Result<crate::domain::Board> {
        self.admit(who, room, Operation::read(room, false))?;
        let now = self.now();
        // statuses and offers are read by prefix at any depth, not from a tail window (#731)
        let messages = self.store.words(room, &crate::domain::BOARD_WORDS)?;
        let roster = self.board_roster(room, now, &messages)?;
        let tasks = self.board_tasks(room, now, &roster.members)?;
        let (queue, merge_holder, halt) = self.merge_queue_of(room, now)?;
        Ok(crate::domain::Board {
            room: room.into(),
            tasks,
            prs: self.open_prs_of(room, now)?,
            queue,
            merge_holder,
            halt,
            asks: self.store.open_asks(room, now)?,
            offers: self.board_offers(&messages, now, &roster),
            members: roster.members,
            stale_members: roster.stale,
        })
    }

    /// One member's card (#251, the design item for #251): what the room board and the live claims say about `member`, or `None` when
    /// it is not listed on the board. Derived on every call, stored nowhere, and a read like the board is.
    pub fn member_card(
        &self,
        who: &Principal,
        room: &str,
        member: &str,
    ) -> Result<Option<crate::domain::Card>> {
        if !valid_name(member) {
            return Err(Error::Invalid("bad member name".into()));
        }
        let board = self.board(who, room)?;
        let now = self.now();
        let claims = self.store.claims(room, now)?;
        let scope = self.store.member_scope(member)?;
        Ok(crate::domain::card_of(
            &board,
            member,
            scope,
            (&claims, now),
        ))
    }
}
