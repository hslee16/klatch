use crate::domain::{
    Claim, Error, MemberInfo, Message, Posted, Principal, Result, RoomDigest, STALE_AFTER_SECS,
    valid_name,
};
use crate::gateway::{Clock, Guard, Operation};
use crate::store::{ClaimRequest, HookEvent, NewMessage, Release, Seat, Store};

/// What a hook needs: unread messages and others' claims per room, and the rooms that were refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digest {
    pub rooms: Vec<RoomDigest>,
    pub refused: Vec<Refused>,
    pub now: i64,
}

/// A room the guard would not let the caller read, with the reason. Shown, never silently dropped (R19).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub room: String,
    pub reason: String,
}

mod admin;
mod anchors;
pub use anchors::{AnchorAdapter, valid_anchor};
mod budget;
mod dm;
mod policy;
mod runs;
pub use runs::{Archive, RunInfo, run_prefix, valid_run_id};
mod scrub;
mod tripwire;
mod workers;
pub use workers::{CredentialInfo, CredentialSpec, MAX_CREDENTIAL_TTL_SECS, check_credential};

const MAX_PATH_BYTES: usize = 1024;
const MAX_REASON_BYTES: usize = 500;

#[cfg(test)]
pub(crate) trait AsSpy {
    fn spy_calls(&self) -> usize;
    fn spy_fail(&self, fail: bool);
}

/// What a claim asks for: the path, how long it should last, why, and whether it is shared with every scope.
#[derive(Debug, Clone, Copy)]
pub struct ClaimSpec<'a> {
    pub path: &'a str,
    pub ttl: i64,
    pub reason: &'a str,
    pub shared: bool,
    /// The work item this claim is for, released when the item is reported done (#125).
    pub anchor: Option<&'a str>,
}

impl<'a> ClaimSpec<'a> {
    pub fn new(path: &'a str, ttl: i64, reason: &'a str) -> Self {
        Self {
            path,
            ttl,
            reason,
            shared: false,
            anchor: None,
        }
    }

    /// The same claim anchored to a work item.
    pub fn anchored(self, anchor: &'a str) -> Self {
        Self {
            anchor: Some(anchor),
            ..self
        }
    }

    /// The same claim, shared with every scope when `shared`.
    pub fn shared(self, shared: bool) -> Self {
        Self { shared, ..self }
    }
}

/// A handoff on a message: a short kind and an optional opaque payload. klatch never reads them.
pub struct Handoff<'a> {
    pub kind: &'a str,
    pub payload: Option<&'a str>,
}

impl Handoff<'_> {
    fn check(&self) -> Result<()> {
        if !crate::domain::valid_kind(self.kind) {
            return Err(Error::Invalid(
                "a handoff kind is 1 to 40 characters of letters, digits, dot, dash and underscore"
                    .into(),
            ));
        }
        if let Some(p) = self.payload {
            if p.is_empty() || p.contains('\u{0}') {
                return Err(Error::Invalid(
                    "a payload is text, not empty and without NUL".into(),
                ));
            }
            if p.len() > crate::domain::MAX_PAYLOAD_BYTES {
                return Err(Error::Invalid(format!(
                    "a payload is at most {} bytes",
                    crate::domain::MAX_PAYLOAD_BYTES
                )));
            }
        }
        Ok(())
    }
}

pub struct Gateway<S: Store> {
    store: S,
    clock: Box<dyn Clock>,
    guard: Box<dyn Guard>,
}

impl<S: Store> Gateway<S> {
    pub fn new(store: S, clock: impl Clock + 'static, guard: impl Guard + 'static) -> Self {
        Self {
            store,
            clock: Box::new(clock),
            guard: Box::new(guard),
        }
    }

    pub fn now(&self) -> i64 {
        self.clock.now()
    }

    #[cfg(test)]
    pub(crate) fn store_calls(&self) -> usize
    where
        S: AsSpy,
    {
        self.store.spy_calls()
    }

    #[cfg(test)]
    pub(crate) fn set_store_failing(&self, fail: bool)
    where
        S: AsSpy,
    {
        self.store.spy_fail(fail)
    }

    pub fn join(&self, who: &Principal, room: &str) -> Result<()> {
        self.admit(who, room, Operation::Join { room: room.into() })?;
        self.store.join(room, &who.member, self.now())
    }

    pub fn post(&self, who: &Principal, room: &str, body: &str) -> Result<Posted> {
        self.post_labeled(who, room, body, None)
    }

    /// Post under a label: the room's own unless `label` says a lower one.
    pub fn post_labeled(
        &self,
        who: &Principal,
        room: &str,
        body: &str,
        label: Option<&str>,
    ) -> Result<Posted> {
        self.post_with(who, (room, body, label), None)
    }

    /// Post a message that carries a handoff: a kind and, optionally, an opaque payload for the next member. The
    /// payload is held to every rule the body is (credentials, canaries, private terms, the label of the room) and is
    /// sealed with the body under a sealed label.
    pub fn post_handoff(
        &self,
        who: &Principal,
        room: &str,
        body: &str,
        label: Option<&str>,
        handoff: &Handoff,
    ) -> Result<Posted> {
        self.post_with(who, (room, body, label), Some(handoff))
    }

    fn post_with(
        &self,
        who: &Principal,
        (room, body, label): (&str, &str, Option<&str>),
        handoff: Option<&Handoff>,
    ) -> Result<Posted> {
        check_member(who)?;
        check_room(room)?;
        if let Some(h) = handoff {
            h.check()?;
        }
        if body.trim().is_empty() {
            return Err(Error::Invalid(
                "empty message: there is nothing to post".into(),
            ));
        }
        self.refuse_text(who, (room, "post"), body)?;
        if let Some(payload) = handoff.and_then(|h| h.payload) {
            self.refuse_text(who, (room, "post"), payload)?;
        }
        self.authorize(
            who,
            &Operation::Post {
                room: room.into(),
                body: body.into(),
            },
        )?;
        let label = self.post_label(who, room, label)?;
        self.check_budget(who, room, body)?;
        let message = NewMessage::new((room, &who.member), who.attribution, &label, body);
        let message = match handoff {
            Some(Handoff {
                kind,
                payload: Some(p),
            }) => message.with_handoff(kind, p),
            Some(Handoff {
                kind,
                payload: None,
            }) => message.with_kind(kind),
            None => message,
        };
        self.store.post_as(&message, self.now())
    }

    /// Remember that the caller is a child of `parent`, the member whose session started the caller's (#24); `None`
    /// records that a parent was looked for and there is none, so it is not looked for again. The first answer noted
    /// is kept.
    pub fn note_parent(&self, who: &Principal, parent: Option<&str>) -> Result<()> {
        check_member(who)?;
        let parent = parent.unwrap_or("");
        let acceptable = parent.is_empty() || (valid_name(parent) && parent != who.member);
        invalid(acceptable, "bad parent member".into())?;
        self.store.note_parent(&who.member, parent, self.now())
    }

    /// The rooms the caller has joined, without reading anything: a hook that only needs to act on each room (a
    /// session ending) must not use up the messages a digest would show the member's next session.
    pub fn rooms(&self, who: &Principal) -> Result<Vec<String>> {
        check_member(who)?;
        self.store.rooms_of(&who.member)
    }

    /// Mute or unmute a room for the caller: its digest then shows only messages that mention it (#20). An explicit
    /// `read` still returns everything.
    pub fn set_muted(&self, who: &Principal, room: &str, muted: bool) -> Result<()> {
        check_member(who)?;
        check_room(room)?;
        self.authorize(
            who,
            &Operation::Mute {
                room: room.into(),
                muted,
            },
        )?;
        self.store
            .set_muted(Seat::new(room, &who.member), muted, self.now())
    }

    pub fn read(&self, who: &Principal, room: &str, advance: bool) -> Result<Vec<Message>> {
        self.admit(who, room, Operation::read(room, advance))?;
        self.store
            .read(Seat::new(room, &who.member), advance, self.now())
    }

    /// What a `wait` should consider: unread messages it has not yet looked at. Moves no cursor; refreshes presence.
    pub fn wait_poll(&self, who: &Principal, room: &str) -> Result<Vec<Message>> {
        self.admit(who, room, Operation::read(room, false))?;
        self.store.wait_poll(room, &who.member, self.now())
    }

    /// How many unread messages would wake the caller in a room: from others, past both cursors, and only those
    /// that mention the caller when it muted the room. Moves no cursor; refreshes presence, so a session that sits
    /// in a watcher is not taken for gone (#37, #39).
    pub fn wakeable(&self, who: &Principal, room: &str) -> Result<usize> {
        self.admit(who, room, Operation::read(room, false))?;
        let unread = self.store.wake_poll(room, &who.member, self.now())?;
        if !self.store.is_muted(Seat::new(room, &who.member))? {
            return Ok(unread.len());
        }
        Ok(unread
            .iter()
            .filter(|m| crate::domain::mentions(&m.body, &who.member))
            .count())
    }

    /// Record that a watcher pointed at everything unread in the room, so it does not wake the caller for those
    /// again. The messages stay unread, and `wait` and `read` still see them.
    pub fn wake_mark(&self, who: &Principal, room: &str) -> Result<()> {
        self.admit(who, room, Operation::read(room, false))?;
        let now = self.now();
        let unread = self.store.wake_poll(room, &who.member, now)?;
        match unread.iter().map(|m| m.seq).max() {
            Some(last) => self
                .store
                .wake_seen(Seat::new(room, &who.member), last, now),
            None => Ok(()),
        }
    }

    /// Record that a `wait` looked at the room up to `seq`; the messages stay unread.
    pub fn wait_seen(&self, who: &Principal, room: &str, seq: i64) -> Result<()> {
        self.admit(who, room, Operation::read(room, false))?;
        self.store
            .wait_seen(Seat::new(room, &who.member), seq, self.now())
    }

    /// The messages a hook shows for a room: all of them, or only those that mention the caller when it muted the
    /// room. A hook shows each message once and leaves the read cursor alone, so a muted-out message stays readable.
    fn digest_messages(&self, who: &Principal, room: &str, now: i64) -> Result<Vec<Message>> {
        let messages = self.store.hook_read(room, &who.member, now)?;
        if !self.store.is_muted(Seat::new(room, &who.member))? {
            return Ok(messages);
        }
        Ok(messages
            .into_iter()
            .filter(|m| crate::domain::mentions(&m.body, &who.member))
            .collect())
    }

    /// Set the caller's own status in a room (for example `done`). Short, one line, no control characters.
    pub fn set_status(&self, who: &Principal, room: &str, status: &str) -> Result<()> {
        check_member(who)?;
        check_room(room)?;
        check_status(status)?;
        self.refuse_text(who, (room, "status"), status)?;
        self.authorize(
            who,
            &Operation::Status {
                room: room.into(),
                status: status.into(),
            },
        )?;
        self.store
            .set_status(Seat::new(room, &who.member), status.trim(), self.now())
    }

    /// Record that one of the caller's hook events ran, and how many bytes it printed (never the content).
    pub fn record_hook_run(&self, who: &Principal, event: &str, bytes: usize) -> Result<()> {
        check_member(who)?;
        check_event_name(event)?;
        self.store.record_hook_run(
            &HookEvent::new(&who.member, event, bytes as i64),
            self.now(),
        )
    }

    /// The latest run of each hook event recorded for `member`: the caller's own freely, anyone else's through the guard.
    pub fn hook_runs(&self, who: &Principal, member: &str) -> Result<Vec<crate::domain::HookRun>> {
        check_member(who)?;
        if !valid_name(member) {
            return Err(Error::Invalid("bad member name".into()));
        }
        // a member reads its own record freely; another member's goes through the guard (#161)
        if member != who.member {
            self.guard
                .check(who, &Operation::admin("hook_runs", member))?;
        }
        self.store.hook_runs(member)
    }

    /// Remember that the caller is the member of harness session `session_id`.
    pub fn note_session(&self, who: &Principal, session_id: &str) -> Result<()> {
        check_member(who)?;
        check_session_id(session_id)?;
        self.store.note_session(session_id, &who.member, self.now())
    }

    /// Housekeeping at session start: remove members not seen for `older_than` seconds. Returns how many.
    pub fn prune_members(&self, who: &Principal, older_than: i64) -> Result<usize> {
        check_member(who)?;
        self.guard
            .check(who, &Operation::admin("prune", &older_than.to_string()))?;
        self.store.prune_members(older_than, self.now())
    }

    /// The room's board: open asks, current offers and members with their last announced status.
    pub fn board(&self, who: &Principal, room: &str) -> Result<crate::domain::Board> {
        self.admit(who, room, Operation::read(room, false))?;
        let now = self.now();
        let messages = self.store.tail(room, 0, 5000)?;
        let statuses = crate::domain::statuses(&messages);
        let all = self.store.members(room)?;
        let done: Vec<String> = all
            .iter()
            .filter(|m| m.status.as_deref().is_some_and(|s| s.starts_with("done")))
            .map(|m| m.name.clone())
            .collect();
        let (recent, gone): (Vec<_>, Vec<_>) = all
            .into_iter()
            .partition(|m| now - m.last_seen <= crate::domain::MEMBER_ACTIVE_SECS);
        // An offer follows its sender's presence: a member that has gone quiet is not available, and a member
        // that keeps waiting (which refreshes presence) keeps its offer with no renewal.
        let absent: Vec<String> = gone.iter().map(|m| m.name.clone()).collect();
        let members = recent
            .into_iter()
            .map(|m| crate::domain::BoardMember {
                status: m.status.clone().or_else(|| {
                    statuses
                        .iter()
                        .find(|s| s.member == m.name)
                        .map(|s| s.status.clone())
                }),
                name: m.name,
                last_seen: m.last_seen,
                parent: m.parent,
            })
            .collect();
        Ok(crate::domain::Board {
            room: room.into(),
            asks: self.store.open_asks(room, now)?,
            offers: crate::domain::current_offers(&messages, now, crate::domain::OFFER_TTL_SECS)
                .into_iter()
                .filter(|o| !done.contains(&o.sender) && !absent.contains(&o.sender))
                .collect(),
            members,
            stale_members: gone.len(),
        })
    }

    /// Asks in a room nobody has answered or closed.
    pub fn open_asks(&self, who: &Principal, room: &str) -> Result<Vec<crate::domain::OpenAsk>> {
        self.admit(who, room, Operation::read(room, false))?;
        self.store.open_asks(room, self.now())
    }

    pub fn who(&self, who: &Principal, room: &str) -> Result<Vec<MemberInfo>> {
        self.admit(who, room, Operation::Who { room: room.into() })?;
        self.store.members(room)
    }

    /// Take a claim. A shared claim, like any `ask:<N>`, conflicts with every scope.
    pub fn claim(&self, who: &Principal, room: &str, spec: &ClaimSpec) -> Result<()> {
        let ClaimSpec {
            path,
            ttl,
            reason,
            shared,
            anchor,
        } = *spec;
        check_member(who)?;
        check_room(room)?;
        check_path(path)?;
        check_reason(reason)?;
        if let Some(a) = anchor
            && !valid_anchor(a)
        {
            return Err(Error::Invalid(format!("bad anchor: {a:?}")));
        }
        self.refuse_text(who, (room, "claim"), reason)?;
        if ttl <= 0 {
            return Err(Error::Invalid("ttl must be positive".into()));
        }
        self.authorize(
            who,
            &Operation::Claim {
                room: room.into(),
                path: path.into(),
            },
        )?;
        let ttl = crate::domain::claim_ttl(path, Some(ttl));
        let member_scope = self.store.member_scope(&who.member)?;
        let scope = crate::domain::claim_scope(path, shared, member_scope.as_deref());
        self.store.claim(
            &ClaimRequest::new((room, &who.member), path, ttl, reason)
                .scoped(&scope)
                .anchored(anchor.unwrap_or("")),
            self.now(),
        )
    }

    /// Declare the scope the caller works in (a repository, a bot's area, a project): claims then conflict only
    /// inside one scope (DESIGN section 27).
    pub fn set_scope(&self, who: &Principal, scope: &str) -> Result<()> {
        check_member(who)?;
        if !crate::domain::valid_scope(scope) {
            return Err(Error::Invalid(
                "bad scope: 1 to 80 characters of letters, digits and . : _ - /".into(),
            ));
        }
        self.store.set_member_scope(&who.member, scope, self.now())
    }

    /// The scope the caller declared or was given, if any.
    pub fn scope_of(&self, who: &Principal) -> Result<Option<String>> {
        check_member(who)?;
        self.store.member_scope(&who.member)
    }

    pub fn release(&self, who: &Principal, room: &str, path: &str) -> Result<()> {
        check_member(who)?;
        check_room(room)?;
        check_path(path)?;
        self.authorize(who, &Operation::release(room, path))?;
        self.store.release(room, &who.member, path)
    }

    /// `release` with a note on what the caller changed (#272): shown to members who saw the claim and returned to
    /// the next claimant. A note is one line of at most 300 characters.
    pub fn release_with_note(
        &self,
        who: &Principal,
        (room, path): (&str, &str),
        note: &str,
    ) -> Result<()> {
        check_member(who)?;
        check_room(room)?;
        check_path(path)?;
        self.refuse_text(who, (room, "release"), note)?;
        let note = crate::domain::flat(note);
        if note.chars().count() > 300 {
            return Err(Error::Invalid(
                "a release note is at most 300 characters".into(),
            ));
        }
        self.authorize(who, &Operation::release(room, path))?;
        self.store.release_with(
            &Release::new((room, &who.member), path, note.trim()),
            self.now(),
        )
    }

    /// The latest release of a path that carried a note.
    pub fn last_release(
        &self,
        who: &Principal,
        room: &str,
        path: &str,
    ) -> Result<Option<crate::domain::ReleaseNote>> {
        self.admit(who, room, Operation::claims(room))?;
        check_path(path)?;
        self.store.last_release(room, path)
    }

    /// Release every claim the caller holds, in every room (its session ended, #277). Only ever the caller's own.
    pub fn release_all(&self, who: &Principal) -> Result<usize> {
        check_member(who)?;
        self.store.release_member_claims(&who.member, self.now())
    }

    pub fn claims(&self, who: &Principal, room: &str) -> Result<Vec<Claim>> {
        self.admit(who, room, Operation::claims(room))?;
        self.store.claims(room, self.now())
    }

    /// Messages after `after_seq`, oldest first, at most `limit`, from every sender including the caller.
    /// No cursor or presence side effects; the guard sees a `Subscribe` operation.
    pub fn tail(
        &self,
        who: &Principal,
        room: &str,
        (after_seq, limit): (i64, usize),
    ) -> Result<Vec<Message>> {
        check_member(who)?;
        check_room(room)?;
        if after_seq < 0 || limit == 0 {
            return Err(Error::Invalid("bad tail range".into()));
        }
        self.guard
            .check(who, &Operation::Subscribe { room: room.into() })?;
        self.store.tail(room, after_seq, limit)
    }

    /// The room's highest `seq`, 0 when empty; the guard sees a `Subscribe` operation.
    pub fn head(&self, who: &Principal, room: &str) -> Result<i64> {
        self.admit(who, room, Operation::Subscribe { room: room.into() })?;
        self.store.head(room)
    }

    /// Acknowledge messages up to `seq`: moves the caller's read cursor forward, never past the head.
    pub fn ack(&self, who: &Principal, room: &str, seq: i64) -> Result<()> {
        check_member(who)?;
        check_room(room)?;
        if seq < 0 {
            return Err(Error::Invalid("bad acknowledgement".into()));
        }
        self.guard
            .check(who, &Operation::Ack { room: room.into() })?;
        self.store
            .advance_cursor(Seat::new(room, &who.member), seq, self.now())
    }

    /// Unread messages and others' claims for every room the caller has joined. A room the guard refuses
    /// is reported in `refused`, never dropped silently; store failures are errors.
    pub fn digest(&self, who: &Principal) -> Result<Digest> {
        self.digest_with(who, false)
    }

    /// `digest`, optionally with the backlog of open unclaimed asks from others (for a session start).
    pub fn digest_with(&self, who: &Principal, backlog: bool) -> Result<Digest> {
        check_member(who)?;
        let now = self.now();
        let mut rooms = Vec::new();
        let mut refused = Vec::new();
        for room in self.store.rooms_of(&who.member)? {
            let op = Operation::Read {
                room: room.clone(),
                advance: true,
            };
            if let Err(e) = self.authorize_quietly(who, &op) {
                refused.push(Refused {
                    room,
                    reason: e.to_string(),
                });
                continue;
            }
            let messages = self.digest_messages(who, &room, now)?;
            let changes = self.store.claim_changes(&room, &who.member, now)?;
            let stale =
                self.store
                    .stale_asks(Seat::new(&room, &who.member), now, STALE_AFTER_SECS)?;
            let escalated = self.store.escalated_asks(&room, &who.member, now)?;
            let lapsed = self.store.lapsed_asks(&room, &who.member, now)?;
            let backlog = if backlog {
                self.store
                    .open_asks(&room, now)?
                    .into_iter()
                    .filter(|a| a.sender != who.member && !a.claimed)
                    .collect()
            } else {
                Vec::new()
            };
            rooms.push(RoomDigest {
                room,
                messages,
                claims: changes.added,
                released: changes.released,
                stale,
                backlog,
                escalated,
                lapsed,
            });
        }
        Ok(Digest {
            rooms,
            refused,
            now,
        })
    }

    /// The administrative rules (`klatch admin`, R11) and then the guard, before the store is touched: a member
    /// removed from a room, a paused gateway and a closed room each refuse with the rule that did.
    fn authorize(&self, who: &Principal, op: &Operation) -> Result<()> {
        self.authorize_quietly(who, op).inspect_err(|e| {
            self.note_refusal(who, op.room().unwrap_or(""), op.name(), e);
        })
    }

    /// `authorize` without a refusal record, for the hook digest: it reports a room it cannot show on every run, and
    /// that is not an attempt by the member.
    fn authorize_quietly(&self, who: &Principal, op: &Operation) -> Result<()> {
        if let Some(room) = op.room() {
            self.check_stops(who, room, op)?;
            self.check_run(room)?;
            self.check_dm(who, room)?;
            if !matches!(op, Operation::Release { .. }) {
                self.check_clearance(who, room)?;
            }
        }
        self.guard.check(who, op)
    }

    /// Validate the caller and the room, then ask the guard. Runs before the store is touched.
    fn admit(&self, who: &Principal, room: &str, op: Operation) -> Result<()> {
        check_member(who)?;
        check_room(room)?;
        self.authorize(who, &op)
    }
}

fn check_member(who: &Principal) -> Result<()> {
    if valid_name(&who.member) {
        Ok(())
    } else {
        Err(Error::Invalid("bad member name".into()))
    }
}

fn check_room(room: &str) -> Result<()> {
    if valid_name(room) {
        Ok(())
    } else {
        Err(Error::Invalid("bad room name".into()))
    }
}

fn has_control(s: &str) -> bool {
    s.chars().any(char::is_control)
}

/// Text of at most `max` bytes with no control character.
fn plain_text(s: &str, max: usize) -> bool {
    s.len() <= max && !has_control(s)
}

/// An identifier: 1 to `max` characters, each alphanumeric or one of `extra`.
fn identifier(s: &str, max: usize, extra: &str) -> bool {
    let allowed = |c: char| c.is_ascii_alphanumeric() || extra.contains(c);
    !s.is_empty() && s.len() <= max && s.chars().all(allowed)
}

fn invalid(unless: bool, what: String) -> Result<()> {
    if unless {
        Ok(())
    } else {
        Err(Error::Invalid(what))
    }
}

/// Refuse text that carries a credential (R20): a room is a log other agents and people read, not a place for secrets.
/// The refusal names the kind of credential and never the text.
fn check_no_credentials(text: &str) -> Result<()> {
    match crate::domain::credential_kind(text) {
        None => Ok(()),
        Some(kind) => Err(Error::Refused {
            rule: "credentials".into(),
            reason: format!(
                "the text looks like it contains {kind}; do not post secrets to a room, keep them in a secret store and say where"
            ),
        }),
    }
}

fn check_status(status: &str) -> Result<()> {
    let ok = !status.trim().is_empty() && status.chars().count() <= 80 && !has_control(status);
    invalid(
        ok,
        "bad status: one line of 1 to 80 characters, no control characters".into(),
    )
}

fn check_event_name(event: &str) -> Result<()> {
    invalid(identifier(event, 40, ""), "bad hook event name".into())
}

fn check_session_id(session_id: &str) -> Result<()> {
    invalid(identifier(session_id, 80, "-_."), "bad session id".into())
}

fn check_path(path: &str) -> Result<()> {
    let ok = !path.is_empty() && plain_text(path, MAX_PATH_BYTES);
    invalid(
        ok,
        format!("bad claim path: 1 to {MAX_PATH_BYTES} bytes, no control characters"),
    )
}

fn check_reason(reason: &str) -> Result<()> {
    invalid(
        plain_text(reason, MAX_REASON_BYTES),
        format!("bad claim reason: at most {MAX_REASON_BYTES} bytes, no control characters"),
    )
}

#[cfg(test)]
mod tests;
