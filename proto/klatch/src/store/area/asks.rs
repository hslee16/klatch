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
        let mut refs = if asks.is_empty() {
            Vec::new()
        } else {
            self.ask_refs(room)?
        };
        for a in &mut asks {
            a.scope = scopes
                .iter()
                .find(|(seq, _)| *seq == a.seq)
                .map(|(_, scope)| scope.clone());
            a.refs = refs
                .iter()
                .position(|(seq, _)| *seq == a.seq)
                .map(|i| refs.swap_remove(i).1)
                .unwrap_or_default();
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

    /// Remember the refs the asker attached to ask `seq` of `room` (#1318), in order. A second call replaces the first;
    /// an empty list clears them. The default remembers nothing.
    fn set_ask_refs(&self, _room: &str, _seq: i64, _refs: &[crate::domain::AskRef]) -> Result<()> {
        Ok(())
    }

    /// The refs of each ask of `room` that carries any, as (seq, refs) pairs, each in the order given (#1318). An ask
    /// with none has no pair. The default knows none.
    fn ask_refs(&self, _room: &str) -> Result<Vec<(i64, Vec<crate::domain::AskRef>)>> {
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

    /// Record that ask `seq` of `room` was routed (#1407): the members it is addressed to, each with its kind, and the
    /// `routed` event with their count (zero is a real answer: nobody capable). Only the first call for an ask
    /// records anything; whether it did. Ids, member names and numbers, never text. The default records nothing.
    fn route_ask(
        &self,
        _room: &str,
        _seq: i64,
        _to: &[(String, crate::domain::RouteKind)],
        _now: i64,
    ) -> Result<bool> {
        Ok(false)
    }

    /// Address ask `seq` to more members after it was routed (the coordinators, once the lease ran out), without
    /// touching the earlier ones. The default records nothing.
    fn add_ask_routes(
        &self,
        _room: &str,
        _seq: i64,
        _to: &[(String, crate::domain::RouteKind)],
        _now: i64,
    ) -> Result<()> {
        Ok(())
    }

    /// Every addressing recorded for `room`, oldest ask first (#1407). The default knows none.
    fn ask_routes(&self, _room: &str) -> Result<Vec<crate::domain::AskRoute>> {
        Ok(Vec::new())
    }

    /// Record a routing outcome (`taken`, `lapsed` or `escalated`) of ask `seq`, once per ask and outcome, with the
    /// number of members it concerns; whether this call was the first. The default records nothing.
    fn note_route_outcome(
        &self,
        _room: &str,
        _seq: i64,
        _outcome: &str,
        _n: i64,
        _now: i64,
    ) -> Result<bool> {
        Ok(false)
    }

    /// The outcomes recorded for ask `seq` of `room`, as words. The default knows none.
    fn route_outcomes(&self, _room: &str, _seq: i64) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    /// The asks of `room` that were addressed to `member` as a coordinator, after the ask was posted, and that surface
    /// `bit` (1 the hook digest, 2 the wake count) has not pointed out to it yet. The message itself is behind the
    /// member's cursors by then, so a later addressing needs this one more way to reach it. The default knows none.
    fn untold_routes(&self, _room: &str, _member: &str, _bit: i64) -> Result<Vec<i64>> {
        Ok(Vec::new())
    }

    /// Remember that surface `bit` pointed out these asks to `member`. The default remembers nothing.
    fn mark_told(&self, _room: &str, _member: &str, _seqs: &[i64], _bit: i64) -> Result<()> {
        Ok(())
    }

    /// Every routing event recorded for `room` as (seq, outcome, ts), by ask and time (#1407). The default knows none.
    fn route_events(&self, _room: &str) -> Result<Vec<(i64, String, i64)>> {
        Ok(Vec::new())
    }
}
