//! klatch-store: the storage seam. Everything above this crate talks to `Store`, never to a database,
//! so SQLite can later be swapped (DESIGN.md D2, R31). Time is passed in so behaviour is testable.

mod ids;
mod sqlite;

pub use sqlite::SqliteStore;

use klatch_core::{
    Attribution, Claim, ClaimChanges, HookRun, MemberInfo, Message, OpenAsk, Posted, Result,
    open_asks,
};

pub trait Store {
    /// Join a room (idempotent). Unread starts at the beginning of history.
    fn join(&self, room: &str, member: &str, now: i64) -> Result<()>;
    /// Append a message, recording whether its sender was proven and the label it is posted under; also marks
    /// the sender as present. Returns its unique id and its per-room `seq` (dense, starting at 1).
    fn post_as(
        &self,
        room: &str,
        sender: &str,
        attribution: Attribution,
        label: &str,
        body: &str,
        now: i64,
    ) -> Result<Posted>;

    /// `post_as` for a sender that is not proven, under the lowest label: the cautious default. The gateway
    /// never uses this; it passes the caller's real attribution.
    fn post(&self, room: &str, sender: &str, body: &str, now: i64) -> Result<Posted> {
        self.post_as(room, sender, Attribution::Unattributed, "public", body, now)
    }
    /// Messages from others after the member's cursor. `advance` moves the cursor to the room's end.
    fn read(&self, room: &str, member: &str, advance: bool, now: i64) -> Result<Vec<Message>>;
    fn members(&self, room: &str) -> Result<Vec<MemberInfo>>;
    /// Record the member's status in a room (replacing any earlier one); joins the member if needed. The default
    /// stores nothing.
    fn set_status(&self, _room: &str, _member: &str, _status: &str, _now: i64) -> Result<()> {
        Ok(())
    }
    /// Take an advisory claim; refuse with `Error::Conflict` if another member holds an overlapping one.
    /// Expired claims are purged first. Re-claiming your own path replaces it.
    fn claim(
        &self,
        room: &str,
        owner: &str,
        path: &str,
        ttl: i64,
        reason: &str,
        now: i64,
    ) -> Result<()>;
    /// Release a claim you hold; `Error::NotFound` if you hold none on that exact path.
    fn release(&self, room: &str, owner: &str, path: &str) -> Result<()>;
    /// Unexpired claims in a room, ordered by path.
    fn claims(&self, room: &str, now: i64) -> Result<Vec<Claim>>;
    /// Asks nobody has answered or closed (DESIGN section 25.4), oldest first. A released claim does not end one.
    fn open_asks(&self, room: &str, now: i64) -> Result<Vec<OpenAsk>> {
        let messages = self.tail(room, 0, 5000)?;
        Ok(open_asks(&messages, &self.claims(room, now)?))
    }
    /// `member`'s own open asks that have had no claim for at least `after` seconds and were not reported to
    /// it before; remembers what it returned. The default remembers nothing and reports them every time.
    fn stale_asks(&self, room: &str, member: &str, now: i64, after: i64) -> Result<Vec<OpenAsk>> {
        Ok(self
            .open_asks(room, now)?
            .into_iter()
            .filter(|a| a.sender == member && !a.claimed && now - a.ts >= after)
            .collect())
    }
    /// Messages from others past both the read cursor and the member's wait position. Changes no cursor, but
    /// refreshes the member's presence, as a read does.
    /// The default is `read` without advancing, which has no wait position.
    fn wait_poll(&self, room: &str, member: &str, now: i64) -> Result<Vec<Message>> {
        self.read(room, member, false, now)
    }
    /// Remember that a wait has looked at the room up to `seq` (forward only), so the next wait does not wake for
    /// the same messages. The read cursor is untouched: the messages stay unread. The default remembers nothing.
    fn wait_seen(&self, _room: &str, _member: &str, _seq: i64, _now: i64) -> Result<()> {
        Ok(())
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
    /// Record that a hook event ran for `member`: the time, and how many bytes it printed (never the content).
    /// The default records nothing.
    fn record_hook_run(&self, _member: &str, _event: &str, _now: i64, _bytes: i64) -> Result<()> {
        Ok(())
    }
    /// The latest run of each hook event recorded for `member`, ordered by event name.
    fn hook_runs(&self, _member: &str) -> Result<Vec<HookRun>> {
        Ok(Vec::new())
    }
    /// Messages from others that a hook has not shown yet, for the member's hook only. Remembers what it
    /// returned so the next hook call does not repeat it, but leaves the member's read cursor alone, so a
    /// message a hook showed can still be read once with `read` (#198). The default advances the read cursor.
    fn hook_read(&self, room: &str, member: &str, now: i64) -> Result<Vec<Message>> {
        self.read(room, member, true, now)
    }
    /// Other members' claims in a room that `member` has not been shown yet, and claims it was shown that are
    /// gone; remembers what it returned so the next call reports only what changed. A claim renewed with a new
    /// expiry counts as changed; the passing of time alone does not. The default remembers nothing and reports
    /// every other member's claim each time.
    fn claim_changes(&self, room: &str, member: &str, now: i64) -> Result<ClaimChanges> {
        Ok(ClaimChanges {
            added: self
                .claims(room, now)?
                .into_iter()
                .filter(|c| c.owner != member)
                .collect(),
            released: Vec::new(),
        })
    }
    /// Rooms a member has joined, ordered by name.
    fn rooms_of(&self, member: &str) -> Result<Vec<String>>;
    /// Messages with `seq` greater than `after_seq`, oldest first, at most `limit`, from every sender
    /// (including the caller, so `seq` has no gaps). No cursor and no presence side effects.
    fn tail(&self, room: &str, after_seq: i64, limit: usize) -> Result<Vec<Message>>;
    /// The highest `seq` in a room, 0 when it is empty.
    fn head(&self, room: &str) -> Result<i64>;
    /// Move a member's read cursor forward to `seq`, never backward and never past the room's head.
    fn advance_cursor(&self, room: &str, member: &str, seq: i64, now: i64) -> Result<()>;
}
