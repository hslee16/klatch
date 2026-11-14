//! The store module (the `klatch-store` crate before the crates became modules): the storage seam. Everything above this crate talks to `Store`, never to a database,
//! so SQLite can later be swapped (DESIGN.md D2, R31). Time is passed in so behaviour is testable.

pub mod encrypt;
mod ids;
mod sqlite;

pub use sqlite::SqliteStore;

use crate::domain::{
    AskRoom, Attribution, AuditRecord, Claim, ClaimChanges, HookRun, MemberInfo, Message, OpenAsk,
    Posted, ReleaseNote, Result, open_asks,
};

/// A member in a room: who is asking, and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seat<'a> {
    pub room: &'a str,
    pub member: &'a str,
}

impl<'a> Seat<'a> {
    pub fn new(room: &'a str, member: &'a str) -> Self {
        Self { room, member }
    }
}

/// A message to append: the room, who sent it, whether the sender was proven, the label it goes under and its text.
#[derive(Debug, Clone, Copy)]
pub struct NewMessage<'a> {
    pub kind: Option<&'a str>,
    pub payload: Option<&'a str>,
    pub room: &'a str,
    pub sender: &'a str,
    pub attribution: Attribution,
    pub label: &'a str,
    pub body: &'a str,
}

impl<'a> NewMessage<'a> {
    pub fn new(
        (room, sender): (&'a str, &'a str),
        attribution: Attribution,
        label: &'a str,
        body: &'a str,
    ) -> Self {
        Self {
            kind: None,
            payload: None,
            room,
            sender,
            attribution,
            label,
            body,
        }
    }

    /// The same message carrying a handoff: a kind and an opaque payload.
    pub fn with_handoff(self, kind: &'a str, payload: &'a str) -> Self {
        Self {
            kind: Some(kind),
            payload: Some(payload),
            ..self
        }
    }

    /// The same message with a kind and no payload.
    pub fn with_kind(self, kind: &'a str) -> Self {
        Self {
            kind: Some(kind),
            ..self
        }
    }
}

/// A request to take a claim. `scope` is empty for a member with none (DESIGN section 27).
#[derive(Debug, Clone, Copy)]
pub struct ClaimRequest<'a> {
    pub room: &'a str,
    pub owner: &'a str,
    pub scope: &'a str,
    pub path: &'a str,
    pub ttl: i64,
    pub reason: &'a str,
    /// The work item the claim is for (a factory's work order, mission or ticket id), empty for none. When the item
    /// is reported done the claim is released.
    pub anchor: &'a str,
}

impl<'a> ClaimRequest<'a> {
    pub fn new(
        (room, owner): (&'a str, &'a str),
        path: &'a str,
        ttl: i64,
        reason: &'a str,
    ) -> Self {
        Self {
            room,
            owner,
            scope: "",
            path,
            ttl,
            reason,
            anchor: "",
        }
    }

    /// The same request anchored to a work item.
    pub fn anchored(self, anchor: &'a str) -> Self {
        Self { anchor, ..self }
    }

    /// The same request inside a scope.
    pub fn scoped(self, scope: &'a str) -> Self {
        Self { scope, ..self }
    }
}

/// What deleting a run's rooms removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeletedRooms {
    pub rooms: usize,
    pub messages: usize,
}

/// A credential to register: the hash of its token (the token itself is never stored), the clearance it carries, the
/// member that launched it and how long it lives.
#[derive(Debug, Clone, Copy)]
pub struct NewCredential<'a> {
    pub member: &'a str,
    pub token_hash: &'a [u8],
    pub clearance: &'a str,
    pub parent: &'a str,
    pub ttl: i64,
}

/// A registered credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialRecord {
    pub member: String,
    pub token_hash: Vec<u8>,
    pub clearance: String,
    pub parent: String,
    pub issued: i64,
    pub expires: i64,
    pub revoked: bool,
}

/// A claim being released, with a note on what the holder changed (empty for none).
#[derive(Debug, Clone, Copy)]
pub struct Release<'a> {
    pub room: &'a str,
    pub owner: &'a str,
    pub path: &'a str,
    pub note: &'a str,
}

impl<'a> Release<'a> {
    pub fn new((room, owner): (&'a str, &'a str), path: &'a str, note: &'a str) -> Self {
        Self {
            room,
            owner,
            path,
            note,
        }
    }
}

/// A hook event that ran for a member, and how many bytes it printed (never the content).
#[derive(Debug, Clone, Copy)]
pub struct HookEvent<'a> {
    pub member: &'a str,
    pub event: &'a str,
    pub bytes: i64,
}

impl<'a> HookEvent<'a> {
    pub fn new(member: &'a str, event: &'a str, bytes: i64) -> Self {
        Self {
            member,
            event,
            bytes,
        }
    }
}

/// An administrative act to keep a record of.
#[derive(Debug, Clone, Copy)]
pub struct NewAudit<'a> {
    pub action: &'a str,
    pub target: &'a str,
    pub detail: &'a str,
}

impl<'a> NewAudit<'a> {
    pub fn new(action: &'a str, target: &'a str, detail: &'a str) -> Self {
        Self {
            action,
            target,
            detail,
        }
    }
}

/// A refused call to keep a record of: who, where, which operation and the rule. Never the refused text.
#[derive(Debug, Clone, Copy)]
pub struct NewRefusal<'a> {
    pub member: &'a str,
    pub room: &'a str,
    pub operation: &'a str,
    pub rule: &'a str,
}

impl<'a> NewRefusal<'a> {
    pub fn new(member: &'a str, room: &'a str, operation: &'a str, rule: &'a str) -> Self {
        Self {
            member,
            room,
            operation,
            rule,
        }
    }
}

pub trait Store {
    /// Join a room (idempotent). Unread starts at the beginning of history.
    fn join(&self, room: &str, member: &str, now: i64) -> Result<()>;
    /// Append a message, recording whether its sender was proven and the label it is posted under; also marks
    /// the sender as present. Returns its unique id and its per-room `seq` (dense, starting at 1).
    fn post_as(&self, message: &NewMessage, now: i64) -> Result<Posted>;

    /// `post_as` for a sender that is not proven, under the lowest label: the cautious default. The gateway
    /// never uses this; it passes the caller's real attribution.
    fn post(&self, at: Seat, body: &str, now: i64) -> Result<Posted> {
        let message = NewMessage::new(
            (at.room, at.member),
            Attribution::Unattributed,
            "public",
            body,
        );
        self.post_as(&message, now)
    }
    /// Messages from others after the member's cursor. `advance` moves the cursor to the room's end.
    fn read(&self, at: Seat, advance: bool, now: i64) -> Result<Vec<Message>>;
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
    fn set_status(&self, _at: Seat, _status: &str, _now: i64) -> Result<()> {
        Ok(())
    }
    /// Take an advisory claim; refuse with `Error::Conflict` if another member holds an overlapping one.
    /// Expired claims are purged first. Re-claiming your own path replaces it. A claim conflicts only with overlapping
    /// claims in the same scope or in the shared scope `*`, and a shared claim conflicts with every scope (DESIGN
    /// section 27).
    fn claim(&self, request: &ClaimRequest, now: i64) -> Result<()>;
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
    /// `release` with a note on what the holder changed (#272), kept for those who saw the claim and for the next
    /// claimant of the path; an empty note clears an older one. The default drops the note.
    fn release_with(&self, release: &Release, _now: i64) -> Result<()> {
        self.release(release.room, release.owner, release.path)
    }
    /// The latest release of `path` that carried a note, if any. The default knows none.
    fn last_release(&self, _room: &str, _path: &str) -> Result<Option<ReleaseNote>> {
        Ok(None)
    }
    /// Set or clear (`None`) a room's administrative state, `closed`; the room `*` carries `paused` for every room
    /// (`klatch admin`, R11). The default remembers nothing.
    fn set_room_state(&self, _room: &str, _state: Option<&str>, _now: i64) -> Result<()> {
        Ok(())
    }
    fn room_state(&self, _room: &str) -> Result<Option<String>> {
        Ok(None)
    }
    /// Remember that `parent` is the member whose session started `member`'s; an empty `parent` records that it was
    /// looked for and there is none. The first one noted is kept. The default stores nothing.
    fn note_parent(&self, _member: &str, _parent: &str, _now: i64) -> Result<()> {
        Ok(())
    }
    /// What was noted for `member`: `None` if nothing, `Some("")` if it was looked for and there is no parent.
    fn parent_of(&self, _member: &str) -> Result<Option<String>> {
        Ok(None)
    }
    /// Mute or unmute a room for a member: a muted member's hook shows only messages that mention it (#20). The
    /// default stores nothing.
    fn set_muted(&self, _at: Seat, _muted: bool, _now: i64) -> Result<()> {
        Ok(())
    }
    /// Whether the member muted the room. The default says no.
    fn is_muted(&self, _at: Seat) -> Result<bool> {
        Ok(false)
    }
    /// Refuse every operation of `member` in `room` (or in every room for `*`) until it is allowed again.
    fn block_member(&self, _room: &str, _member: &str, _now: i64) -> Result<()> {
        Ok(())
    }
    /// Lift a block; whether there was one.
    fn unblock_member(&self, _room: &str, _member: &str) -> Result<bool> {
        Ok(false)
    }
    /// Whether `member` is blocked in `room`, by a block on that room or on every room.
    fn is_blocked(&self, _room: &str, _member: &str) -> Result<bool> {
        Ok(false)
    }
    /// Take `member` out of `room` and release its claims there; returns how many claims were released.
    fn remove_member(&self, _room: &str, _member: &str, _now: i64) -> Result<usize> {
        Ok(0)
    }
    /// Keep a record of an administrative act, and read the latest ones, newest first.
    fn record_audit(&self, _entry: &NewAudit, _now: i64) -> Result<()> {
        Ok(())
    }
    fn audit_log(&self, _limit: usize) -> Result<Vec<AuditRecord>> {
        Ok(Vec::new())
    }
    /// Policy the administrator set: room labels, clearances, the label chain, scrub terms, tripwires (epic #9). One
    /// key and one value each; `None` clears a key. The default remembers nothing.
    fn policy_get(&self, _key: &str) -> Result<Option<String>> {
        Ok(None)
    }
    /// Register a credential for a member. A member that holds a live one (not revoked, not expired) cannot be given
    /// another until it is revoked or lapses. The default registers nothing.
    fn issue_credential(&self, _credential: &NewCredential, _now: i64) -> Result<()> {
        Err(crate::domain::Error::Invalid(
            "this store keeps no credentials".into(),
        ))
    }
    fn credential(&self, _member: &str) -> Result<Option<CredentialRecord>> {
        Ok(None)
    }
    /// Revoke a member's credential; false when there is none or it was already revoked.
    fn revoke_credential(&self, _member: &str, _now: i64) -> Result<bool> {
        Ok(false)
    }
    fn credentials(&self) -> Result<Vec<CredentialRecord>> {
        Ok(Vec::new())
    }
    /// Release every live claim anchored to `anchor` (the work item was reported done) and clear the lapsed ones;
    /// returns `(room, path, owner)` of each live claim released. The default releases nothing.
    fn release_anchored(&self, _anchor: &str, _now: i64) -> Result<Vec<(String, String, String)>> {
        Ok(Vec::new())
    }
    /// The distinct anchors of live claims, sorted: what a poll has to ask about.
    fn anchors_in_use(&self, _now: i64) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
    /// Delete every room whose name starts with `prefix` (matched literally), from every table keyed by room, and the
    /// label policy of those rooms. The record of refusals and the administrative audit are not part of a room and
    /// stay. An empty prefix deletes nothing. The default deletes nothing.
    fn delete_rooms_with_prefix(&self, _prefix: &str, _now: i64) -> Result<DeletedRooms> {
        Ok(DeletedRooms::default())
    }
    /// The label keys this principal holds (DESIGN section 32 item 8). The default ignores them.
    fn set_label_keys(&self, _keys: crate::keys::LabelKeys) {}
    fn policy_set(&self, _key: &str, _value: Option<&str>, _now: i64) -> Result<()> {
        Ok(())
    }
    /// Every policy entry whose key starts with `prefix`, in key order.
    fn policy_list(&self, _prefix: &str) -> Result<Vec<(String, String)>> {
        Ok(Vec::new())
    }
    /// Keep a record of a refused call, and read the latest ones (newest first) or the rules one member was refused
    /// under since a time.
    fn record_refusal(&self, _entry: &NewRefusal, _now: i64) -> Result<()> {
        Ok(())
    }
    fn refusals(&self, _limit: usize) -> Result<Vec<crate::domain::RefusalRecord>> {
        Ok(Vec::new())
    }
    fn refusal_rules_since(&self, _member: &str, _since: i64) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
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
    fn stale_asks(&self, at: Seat, now: i64, after: i64) -> Result<Vec<OpenAsk>> {
        Ok(self
            .open_asks(at.room, now)?
            .into_iter()
            .filter(|a| {
                a.sender == at.member
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
        self.read(Seat::new(room, member), false, now)
    }
    /// Remember that a wait has looked at the room up to `seq` (forward only), so the next wait does not wake for
    /// the same messages. The read cursor is untouched: the messages stay unread. The default remembers nothing.
    fn wait_seen(&self, _at: Seat, _seq: i64, _now: i64) -> Result<()> {
        Ok(())
    }
    /// Messages from others past both the read cursor and the member's wake position (what a watcher has already
    /// pointed at). Changes no position; refreshes presence. The default is `read` without advancing.
    fn wake_poll(&self, room: &str, member: &str, now: i64) -> Result<Vec<Message>> {
        self.read(Seat::new(room, member), false, now)
    }
    /// Remember that a watcher pointed at the room up to `seq` (forward only). The default remembers nothing.
    fn wake_seen(&self, _at: Seat, _seq: i64, _now: i64) -> Result<()> {
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
    fn record_hook_run(&self, _run: &HookEvent, _now: i64) -> Result<()> {
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
        self.read(Seat::new(room, member), true, now)
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
    fn advance_cursor(&self, at: Seat, seq: i64, now: i64) -> Result<()>;
}

/// A fixed key for tests that need a file-backed store. It is public and therefore worthless as a secret: never use
/// it outside tests.
#[doc(hidden)]
pub fn insecure_test_key() -> crate::keys::Key {
    crate::keys::Key::from_bytes([0x5A; 32])
}
