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
        let scopes = if asks.is_empty() {
            Vec::new()
        } else {
            self.ask_scopes(room)?
        };
        for a in &mut asks {
            a.scope = scopes
                .iter()
                .find(|(seq, _)| *seq == a.seq)
                .map(|(_, scope)| scope.clone());
            a.room = rooms
                .iter()
                .find(|r| r.name == format!("ask-{}", a.seq))
                .cloned();
            self.attach_dm(a)?;
        }
        Ok(asks)
    }

    /// Remember the scope the asker declared on ask `seq` of `room` (#1171). A second call replaces the first. The
    /// default remembers nothing.
    fn set_ask_scope(&self, _room: &str, _seq: i64, _scope: &str) -> Result<()> {
        Ok(())
    }

    /// The declared scope of each ask of `room` that has one, as (seq, scope) pairs (#1171). An ask without a
    /// declared scope has no pair. The default knows none.
    fn ask_scopes(&self, _room: &str) -> Result<Vec<(i64, String)>> {
        Ok(Vec::new())
    }

    /// Finds the dm a moved ask points at (#1188). The pointer stands only when that room exists with both members
    /// in it; otherwise it is dropped. Only the room's name, member count and latest timestamp are read, never a
    /// message.
    fn attach_dm(&self, ask: &mut OpenAsk) -> Result<()> {
        let Some(m) = ask.moved.as_mut() else {
            return Ok(());
        };
        let name = crate::domain::dm_room(&m.by, &m.with);
        m.room = self
            .rooms_with_prefix(&name)?
            .into_iter()
            .find(|r| r.name == name && r.members >= 2);
        if m.room.is_none() {
            ask.moved = None;
        }
        Ok(())
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
