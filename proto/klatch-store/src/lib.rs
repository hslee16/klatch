//! klatch-store: the storage seam. Everything above this crate talks to `Store`, never to a database,
//! so SQLite can later be swapped (DESIGN.md D2, R31). Time is passed in so behaviour is testable.

mod ids;
mod sqlite;

pub use sqlite::SqliteStore;

use klatch_core::{
    AskRoom, Attribution, Claim, ClaimChanges, HookRun, MemberInfo, Message, OpenAsk, Posted,
    Result, open_asks,
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
    /// Remember that harness session `session_id` is member `member` (replacing an earlier entry), so a hook that
    /// runs after the harness has dropped its own record can still find its member. The default remembers nothing.
    fn note_session(&self, _session_id: &str, _member: &str, _now: i64) -> Result<()> {
        Ok(())
    }
    /// The harness session that holds a member name, if any: the most recent one noted for it.
    fn session_of_member(&self, _member: &str) -> Result<Option<String>> {
        Ok(None)
    }
    /// The member remembered for a harness session, if any.
    fn member_of_session(&self, _session_id: &str) -> Result<Option<String>> {
        Ok(None)
    }
    /// Remove members of every room not seen for `older_than` seconds, with their per-member bookkeeping (claims
    /// seen, reported asks, hook runs). Messages and claims stay. Returns how many members were removed. The default
    /// removes nothing.
    fn prune_members(&self, _older_than: i64, _now: i64) -> Result<usize> {
        Ok(0)
    }
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
    /// `claim` inside a scope (DESIGN section 27): a claim conflicts only with overlapping claims in the same scope
    /// or in the shared scope `*`, and a shared claim conflicts with every scope. The default ignores the scope.
    #[allow(clippy::too_many_arguments)]
    fn claim_in(
        &self,
        room: &str,
        owner: &str,
        _scope: &str,
        path: &str,
        ttl: i64,
        reason: &str,
        now: i64,
    ) -> Result<()> {
        self.claim(room, owner, path, ttl, reason, now)
    }
    /// The scope a member declared or was given, if any. The default knows none.
    fn member_scope(&self, _member: &str) -> Result<Option<String>> {
        Ok(None)
    }
    /// Remember a member's scope (replacing an earlier one). The default stores nothing.
    fn set_member_scope(&self, _member: &str, _scope: &str, _now: i64) -> Result<()> {
        Ok(())
    }
    /// Release a claim you hold; `Error::NotFound` if you hold none on that exact path.
    fn release(&self, room: &str, owner: &str, path: &str) -> Result<()>;
    /// Release every claim `member` holds in every room, as when its session ends (#277): a member that has ended
    /// cannot be working. An `ask:<N>` claim ends as a lapse, so the asker is told that the helper's claim ended
    /// without an answer. Returns how many unexpired claims were released. The default releases nothing.
    fn release_member_claims(&self, _member: &str, _now: i64) -> Result<usize> {
        Ok(0)
    }
    /// Unexpired claims in a room, ordered by path.
    fn claims(&self, room: &str, now: i64) -> Result<Vec<Claim>>;
    /// Rooms whose name starts with `prefix`, with their member count and latest activity (a member seen or a
    /// message posted). The default knows none.
    fn rooms_with_prefix(&self, _prefix: &str) -> Result<Vec<AskRoom>> {
        Ok(Vec::new())
    }
    /// Asks nobody has answered or closed (DESIGN section 25.4), oldest first. A released claim does not end one.
    /// Each carries its working room, `ask-<N>`, if anyone has joined it.
    fn open_asks(&self, room: &str, now: i64) -> Result<Vec<OpenAsk>> {
        let messages = self.tail(room, 0, 5000)?;
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
    fn stale_asks(&self, room: &str, member: &str, now: i64, after: i64) -> Result<Vec<OpenAsk>> {
        Ok(self
            .open_asks(room, now)?
            .into_iter()
            .filter(|a| {
                a.sender == member
                    && !a.claimed
                    && a.waiting_on_asker.is_none()
                    && !a.room_active(now)
                    && now - a.ts >= after
            })
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
