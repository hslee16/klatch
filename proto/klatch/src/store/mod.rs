//! The store module (the `klatch-store` crate before the crates became modules): the storage seam. Everything above this crate talks to `Store`, never to a database,
//! so SQLite can later be swapped (DESIGN.md D2, R31). Time is passed in so behaviour is testable.

mod area;
#[cfg(test)]
pub mod conformance;
pub mod encrypt;
pub(crate) mod ids;
#[cfg(test)]
mod memory_spike;
mod sqlite;
#[cfg(test)]
pub(crate) mod test_dir;
#[cfg(test)]
mod test_dir_tests;

mod features;
pub use area::{
    AskStore, ClaimStore, CredentialStore, MemberStore, MessageStore, PolicyStore, ReviewStore,
    RoomStore,
};
pub use features::{Feature, Features, require_features};
/// Every area trait in one place, for code that holds a concrete store and calls its methods (a trait's methods need the trait in scope).
pub mod areas {
    pub use super::{
        AskStore, ClaimStore, CredentialStore, MemberStore, MessageStore, PolicyStore, ReviewStore,
        RoomStore, Store,
    };
}
pub use sqlite::{
    ArchiveInfo, ArchiveState, SCHEMA_VERSION, SqliteStore, archives_to_rekey, check_store,
    list_archives, rekey_archives, rekey_archives_in, reseal_archives, resealable_archives,
    wrap_archives,
};

use crate::domain::Attribution;

/// A member in a room: who is asking, and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seat<'a> {
    pub room: &'a str,
    pub member: &'a str,
}

/// A position in a room's log: the per-room `seq` of a message (dense, from 1). A type of its own so a cursor is
/// not confused with a time or a count (#695).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Seq(pub i64);

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
    /// The idle timeout: the credential lives this long from issue and, while it is used, from each use.
    pub ttl: i64,
    /// The hard cap in seconds from issue that renewal never passes; 0 means the credential is never renewed.
    pub max_life: i64,
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
    /// The idle timeout renewal pushes the expiry out by (0 for a credential from before renewal existed).
    pub idle: i64,
    /// The moment renewal never passes (0: never renewed).
    pub cap: i64,
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

/// A message that needs review before it crosses to `to_label`: a pointer, not a copy, so the queue holds no text.
#[derive(Debug, Clone, Copy)]
pub struct NewReview<'a> {
    pub room: &'a str,
    pub seq: i64,
    pub to_label: &'a str,
    pub reason: &'a str,
    /// For a crossing (#497): the room the text would be copied into and the member who asked. Empty otherwise.
    pub to_room: &'a str,
    pub requester: &'a str,
}

impl<'a> NewReview<'a> {
    pub fn new(room: &'a str, seq: i64, to_label: &'a str, reason: &'a str) -> Self {
        Self {
            room,
            seq,
            to_label,
            reason,
            to_room: "",
            requester: "",
        }
    }

    /// Make this item a crossing into `to_room`, asked for by `requester`.
    pub fn crossing(self, to_room: &'a str, requester: &'a str) -> Self {
        Self {
            to_room,
            requester,
            ..self
        }
    }
}

/// An item waiting in the review queue.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueuedReview {
    pub id: i64,
    pub room: String,
    pub seq: i64,
    pub to_label: String,
    pub reason: String,
    pub ts: i64,
    pub to_room: String,
    pub requester: String,
    /// The verdict word once there is one, else empty.
    pub verdict: String,
    pub detail: String,
    /// The text a `redact` verdict supplied: the only text a queue row ever holds.
    pub redacted: String,
    /// What became of the verdict: `released` or `blocked` for a crossing, else empty.
    pub applied: String,
}

impl QueuedReview {
    /// A pointer at a message, with no queue id: enough to load the message's text.
    pub fn pointer(room: &str, seq: i64) -> Self {
        Self {
            room: room.into(),
            seq,
            ..Self::default()
        }
    }
}

/// A verdict on a queued review: which item, the verdict word and the reason recorded with it.
#[derive(Debug, Clone, Copy)]
pub struct ReviewResolution<'a> {
    pub id: i64,
    pub verdict: &'a str,
    pub detail: &'a str,
}

impl<'a> ReviewResolution<'a> {
    pub fn new(id: i64, verdict: &'a str, detail: &'a str) -> Self {
        Self {
            id,
            verdict,
            detail,
        }
    }
}

/// The storage seam: every area of the store in one bound. Callers that need all of it take `Store`; a caller that needs one area can
/// take that area's trait. A store implements the area traits (each has defaults for what it does not keep) and is a `Store` for free.
pub trait Store:
    MessageStore
    + MemberStore
    + ClaimStore
    + AskStore
    + ReviewStore
    + CredentialStore
    + PolicyStore
    + RoomStore
{
}

impl<T> Store for T where
    T: MessageStore
        + MemberStore
        + ClaimStore
        + AskStore
        + ReviewStore
        + CredentialStore
        + PolicyStore
        + RoomStore
{
}

/// A fixed key for tests that need a file-backed store. It is public and therefore worthless as a secret: never use
/// it outside tests.
#[doc(hidden)]
pub fn insecure_test_key() -> crate::keys::Key {
    crate::keys::Key::from_bytes([0x5A; 32])
}
