//! The `AskStore` area of the store seam; `Store` in the parent module combines every area.

use crate::domain::{AskRoom, OpenAsk, Result, open_asks};
use crate::store::Seat;
use crate::store::{ClaimStore, MessageStore};

pub trait AskStore: MessageStore + ClaimStore {
    /// Rooms whose name starts with `prefix`, with their member count and latest activity (a member seen or a
    /// message posted). The default knows none.
    fn rooms_with_prefix(&self, _prefix: &str) -> Result<Vec<AskRoom>> {
        Ok(Vec::new())
    }

    /// Asks nobody has answered or closed (DESIGN section 25.4), oldest first. A released claim does not end one.
    /// Each carries its working room, `ask-<N>`, if anyone has joined it.
    fn open_asks(&self, room: &str, now: i64) -> Result<Vec<OpenAsk>> {
        // by prefix at any depth, not from a tail window: an ask, an answer or a decline can be older than any window
        let messages = self.words(room, &crate::domain::ASK_WORDS)?;
        let mut asks = open_asks(&messages, &self.claims(room, now)?);
        let rooms = self.rooms_with_prefix("ask-")?;
        for a in &mut asks {
            a.room = rooms
                .iter()
                .find(|r| r.name == format!("ask-{}", a.seq))
                .cloned();
        }
        Ok(asks)
    }

    /// `member`'s own open asks whose claim lapsed without an answer (a helper that failed or vanished), not
    /// reported to it before for that lapse, with the holder whose lease ran out; remembers what it returned. An
    /// ask whose claim was released on purpose, or that is claimed again, is not a lapse. The default knows none.
    fn lapsed_asks(&self, _room: &str, _member: &str, _now: i64) -> Result<Vec<(OpenAsk, String)>> {
        Ok(Vec::new())
    }

    /// `member`'s own open asks that have had no claim for at least `after` seconds and were not reported to
    /// it before; remembers what it returned. The default remembers nothing and reports them every time.
    fn stale_asks(&self, at: Seat, now: i64, after: i64) -> Result<Vec<OpenAsk>> {
        Ok(self
            .open_asks(at.room, now)?
            .into_iter()
            .filter(|a| {
                a.sender == at.member
                    && !a.claimed
                    && a.waiting_on_asker.is_none()
                    && !a.room_active(now)
                    && now - a.ts >= a.stale_after(after)
            })
            .collect())
    }

    /// `member`'s own open asks that enough helpers declined as `cannot` (none busy or failed), not reported to it
    /// before; remembers what it returned. The default remembers nothing and reports them every time.
    fn escalated_asks(&self, room: &str, member: &str, now: i64) -> Result<Vec<OpenAsk>> {
        Ok(self
            .open_asks(room, now)?
            .into_iter()
            .filter(|a| a.sender == member && a.no_capable_member())
            .collect())
    }
}
