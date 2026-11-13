//! Test doubles shared by the gateway tests.
use crate::{Guard, Operation};
use klatch_core::{Claim, Error, MemberInfo, Message, Posted, Principal, Result};
use klatch_store::{SqliteStore, Store};
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
    fn post(&self, room: &str, sender: &str, body: &str, now: i64) -> Result<Posted> {
        self.gate()?;
        self.inner.post(room, sender, body, now)
    }
    fn read(&self, room: &str, member: &str, advance: bool, now: i64) -> Result<Vec<Message>> {
        self.gate()?;
        self.inner.read(room, member, advance, now)
    }
    fn members(&self, room: &str) -> Result<Vec<MemberInfo>> {
        self.gate()?;
        self.inner.members(room)
    }
    fn claim(
        &self,
        room: &str,
        owner: &str,
        path: &str,
        ttl: i64,
        reason: &str,
        now: i64,
    ) -> Result<()> {
        self.gate()?;
        self.inner.claim(room, owner, path, ttl, reason, now)
    }
    fn release(&self, room: &str, owner: &str, path: &str) -> Result<()> {
        self.gate()?;
        self.inner.release(room, owner, path)
    }
    fn claims(&self, room: &str, now: i64) -> Result<Vec<Claim>> {
        self.gate()?;
        self.inner.claims(room, now)
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
    fn advance_cursor(&self, room: &str, member: &str, seq: i64, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.advance_cursor(room, member, seq, now)
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
    match op {
        Operation::Join { room }
        | Operation::Post { room, .. }
        | Operation::Read { room, .. }
        | Operation::Who { room }
        | Operation::Claim { room, .. }
        | Operation::Release { room, .. }
        | Operation::Claims { room }
        | Operation::Subscribe { room }
        | Operation::Ack { room } => room,
    }
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
