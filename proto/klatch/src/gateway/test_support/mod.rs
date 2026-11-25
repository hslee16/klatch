//! Test doubles shared by the gateway tests.
use crate::domain::{Claim, Error, MemberInfo, Message, Posted, Principal, Result};
use crate::gateway::{Guard, Operation};
use crate::store::{
    AskStore, ClaimRequest, ClaimStore, CredentialStore, HookEvent, MemberStore, MessageStore,
    NewAudit, NewMessage, PolicyStore, Release, ReviewResolution, ReviewStore, RoomStore, Seat,
    Seq, SqliteStore,
};
use std::cell::{Cell, RefCell};

mod admin;
mod claims;
mod messages;

/// A real in-memory store that counts calls and can be told to fail every one.
pub struct SpyStore {
    inner: SqliteStore,
    pub calls: Cell<usize>,
    pub fail: Cell<bool>,
    /// Fail only the message id to room lookup (#1328), so a test can see a post fail closed on it alone.
    pub fail_message_room: Cell<bool>,
    /// Called with the key just before a policy write reaches the store: a test uses it to make another connection
    /// act in the window between a check and its write (#897).
    pub before_policy_set: RefCell<Option<PolicyHook>>,
}

type PolicyHook = Box<dyn Fn(&str)>;

impl SpyStore {
    pub fn new() -> Self {
        Self::over(SqliteStore::open_in_memory().unwrap())
    }
    /// A spy over a store on disk, so a second connection to the same file can race it.
    pub fn over(inner: SqliteStore) -> Self {
        Self {
            inner,
            calls: Cell::new(0),
            fail: Cell::new(false),
            fail_message_room: Cell::new(false),
            before_policy_set: RefCell::new(None),
        }
    }
    /// The claim event record as the stats report reads it (#1386).
    pub fn claim_history_for_test(&self) -> crate::domain::ClaimHistory {
        let opts = crate::domain::StatsOptions {
            top: 1,
            ..Default::default()
        };
        self.inner.usage_stats(&opts).unwrap().claims.history
    }

    /// The passive claim counts as the stats report reads them (#1401).
    pub fn passive_counts_for_test(&self) -> crate::domain::PassiveClaimCounts {
        let opts = crate::domain::StatsOptions {
            top: 1,
            ..Default::default()
        };
        self.inner.usage_stats(&opts).unwrap().passive_claims
    }

    /// The whole usage report, for a test of what it prints.
    pub fn stats_for_test(&self, opts: &crate::domain::StatsOptions) -> crate::domain::UsageStats {
        self.inner.usage_stats(opts).unwrap()
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

impl crate::gateway::gateway::AsSpy for SpyStore {
    fn spy_calls(&self) -> usize {
        self.calls.get()
    }
    fn spy_fail(&self, fail: bool) {
        self.fail.set(fail);
    }
}
