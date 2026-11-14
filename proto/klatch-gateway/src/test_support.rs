//! Test doubles shared by the gateway tests.
use crate::{Guard, Operation};
use klatch_core::{Claim, Error, MemberInfo, Message, Posted, Principal, Result};
use klatch_store::{
    ClaimRequest, HookEvent, NewAudit, NewMessage, Release, Seat, SqliteStore, Store,
};
use std::cell::{Cell, RefCell};

/// A real in-memory store that counts calls and can be told to fail every one.
pub struct SpyStore {
    inner: SqliteStore,
    pub calls: Cell<usize>,
    pub fail: Cell<bool>,
}

impl SpyStore {
    pub fn new() -> Self {
        Self {
            inner: SqliteStore::open_in_memory().unwrap(),
            calls: Cell::new(0),
            fail: Cell::new(false),
        }
    }
    /// Put a member in a room directly, without going through the gateway.
    pub fn inner_join_for_test(&self, room: &str, member: &str) {
        self.inner.join(room, member, 0).unwrap();
    }
    fn gate(&self) -> Result<()> {
        self.calls.set(self.calls.get() + 1);
        if self.fail.get() {
            Err(Error::Backend("store down".into()))
        } else {
            Ok(())
        }
    }
}

impl Store for SpyStore {
    fn join(&self, room: &str, member: &str, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.join(room, member, now)
    }
    fn post_as(&self, message: &NewMessage, now: i64) -> Result<Posted> {
        self.gate()?;
        self.inner.post_as(message, now)
    }
    fn read(&self, at: Seat, advance: bool, now: i64) -> Result<Vec<Message>> {
        self.gate()?;
        self.inner.read(at, advance, now)
    }
    fn members(&self, room: &str) -> Result<Vec<MemberInfo>> {
        self.gate()?;
        self.inner.members(room)
    }
    fn claim(&self, request: &ClaimRequest, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.claim(request, now)
    }
    fn release(&self, room: &str, owner: &str, path: &str) -> Result<()> {
        self.gate()?;
        self.inner.release(room, owner, path)
    }
    fn set_room_state(&self, room: &str, state: Option<&str>, now: i64) -> Result<()> {
        self.inner.set_room_state(room, state, now)
    }
    fn room_state(&self, room: &str) -> Result<Option<String>> {
        self.inner.room_state(room)
    }
    fn block_member(&self, room: &str, member: &str, now: i64) -> Result<()> {
        self.inner.block_member(room, member, now)
    }
    fn unblock_member(&self, room: &str, member: &str) -> Result<bool> {
        self.inner.unblock_member(room, member)
    }
    fn is_blocked(&self, room: &str, member: &str) -> Result<bool> {
        self.inner.is_blocked(room, member)
    }
    fn remove_member(&self, room: &str, member: &str, now: i64) -> Result<usize> {
        self.inner.remove_member(room, member, now)
    }
    fn record_audit(&self, entry: &NewAudit, now: i64) -> Result<()> {
        self.inner.record_audit(entry, now)
    }
    fn audit_log(&self, limit: usize) -> Result<Vec<klatch_core::AuditRecord>> {
        self.inner.audit_log(limit)
    }
    fn release_with(&self, release: &Release, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.release_with(release, now)
    }
    fn last_release(&self, room: &str, path: &str) -> Result<Option<klatch_core::ReleaseNote>> {
        self.gate()?;
        self.inner.last_release(room, path)
    }
    fn release_member_claims(&self, member: &str, now: i64) -> Result<usize> {
        self.gate()?;
        self.inner.release_member_claims(member, now)
    }
    fn claims(&self, room: &str, now: i64) -> Result<Vec<Claim>> {
        self.gate()?;
        self.inner.claims(room, now)
    }
    fn hook_read(&self, room: &str, member: &str, now: i64) -> Result<Vec<klatch_core::Message>> {
        self.gate()?;
        self.inner.hook_read(room, member, now)
    }
    fn wait_poll(&self, room: &str, member: &str, now: i64) -> Result<Vec<klatch_core::Message>> {
        self.gate()?;
        self.inner.wait_poll(room, member, now)
    }
    fn wait_seen(&self, at: Seat, seq: i64, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.wait_seen(at, seq, now)
    }
    fn stale_asks(&self, at: Seat, now: i64, after: i64) -> Result<Vec<klatch_core::OpenAsk>> {
        self.gate()?;
        self.inner.stale_asks(at, now, after)
    }
    fn open_asks(&self, room: &str, now: i64) -> Result<Vec<klatch_core::OpenAsk>> {
        self.gate()?;
        self.inner.open_asks(room, now)
    }
    fn set_status(&self, at: Seat, status: &str, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.set_status(at, status, now)
    }
    fn record_hook_run(&self, run: &HookEvent, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.record_hook_run(run, now)
    }
    fn hook_runs(&self, member: &str) -> Result<Vec<klatch_core::HookRun>> {
        self.gate()?;
        self.inner.hook_runs(member)
    }
    fn prune_members(&self, older_than: i64, now: i64) -> Result<usize> {
        self.gate()?;
        self.inner.prune_members(older_than, now)
    }
    fn note_session(&self, session_id: &str, member: &str, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.note_session(session_id, member, now)
    }
    fn session_of_member(&self, member: &str) -> Result<Option<String>> {
        self.gate()?;
        self.inner.session_of_member(member)
    }
    fn member_of_session(&self, session_id: &str) -> Result<Option<String>> {
        self.gate()?;
        self.inner.member_of_session(session_id)
    }

    fn member_scope(&self, member: &str) -> Result<Option<String>> {
        self.gate()?;
        self.inner.member_scope(member)
    }
    fn set_member_scope(&self, member: &str, scope: &str, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.set_member_scope(member, scope, now)
    }
    fn lapsed_asks(
        &self,
        room: &str,
        member: &str,
        now: i64,
    ) -> Result<Vec<(klatch_core::OpenAsk, String)>> {
        self.gate()?;
        self.inner.lapsed_asks(room, member, now)
    }
    fn rooms_with_prefix(&self, prefix: &str) -> Result<Vec<klatch_core::AskRoom>> {
        self.gate()?;
        self.inner.rooms_with_prefix(prefix)
    }
    fn claim_changes(
        &self,
        room: &str,
        member: &str,
        now: i64,
    ) -> Result<klatch_core::ClaimChanges> {
        self.gate()?;
        self.inner.claim_changes(room, member, now)
    }
    fn rooms_of(&self, member: &str) -> Result<Vec<String>> {
        self.gate()?;
        self.inner.rooms_of(member)
    }
    fn tail(&self, room: &str, after_seq: i64, limit: usize) -> Result<Vec<Message>> {
        self.gate()?;
        self.inner.tail(room, after_seq, limit)
    }
    fn head(&self, room: &str) -> Result<i64> {
        self.gate()?;
        self.inner.head(room)
    }
    fn advance_cursor(&self, at: Seat, seq: i64, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.advance_cursor(at, seq, now)
    }
}

/// Records every operation it is asked about; denies rooms listed in `deny_rooms`.
pub struct SpyGuard {
    pub seen: RefCell<Vec<Operation>>,
    pub deny_rooms: Vec<String>,
}

impl SpyGuard {
    pub fn allowing() -> Self {
        Self {
            seen: RefCell::new(Vec::new()),
            deny_rooms: Vec::new(),
        }
    }
    pub fn denying(rooms: &[&str]) -> Self {
        Self {
            seen: RefCell::new(Vec::new()),
            deny_rooms: rooms.iter().map(|r| r.to_string()).collect(),
        }
    }
}

fn room_of(op: &Operation) -> &str {
    op.room().unwrap_or("")
}

impl Guard for std::rc::Rc<SpyGuard> {
    fn check(&self, _who: &Principal, op: &Operation) -> Result<()> {
        self.seen.borrow_mut().push(op.clone());
        if self.deny_rooms.iter().any(|r| r == room_of(op)) {
            Err(Error::Refused {
                rule: "test.deny_room".into(),
                reason: format!("room {} is not allowed", room_of(op)),
            })
        } else {
            Ok(())
        }
    }
}

impl crate::gateway::AsSpy for SpyStore {
    fn spy_calls(&self) -> usize {
        self.calls.get()
    }
    fn spy_fail(&self, fail: bool) {
        self.fail.set(fail);
    }
}
