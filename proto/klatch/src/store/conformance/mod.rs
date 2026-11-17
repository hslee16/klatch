//! The store conformance suite (#74): every behaviour a `Store` must have, written against the trait only. A backend runs
//! it by calling each function with a constructor for an empty store; `sqlite/tests/conformance.rs` does so for SQLite.
//! A test that needs a backend's own internals (the file, a raw connection, migrations) stays with that backend.

#![allow(unused_imports)]
use crate::domain::*;
use crate::store::*;

pub(super) fn deadlocked(r: Result<()>) -> Option<bool> {
    match r {
        Err(Error::Conflict { deadlock, .. }) => Some(deadlock),
        _ => None,
    }
}

/// When and for how long a claim is taken, and the work item it is anchored to (none when empty).
#[derive(Clone, Copy)]
pub(super) struct Lease<'a> {
    now: i64,
    ttl: i64,
    anchor: &'a str,
}

impl<'a> Lease<'a> {
    pub(super) fn new(now: i64, ttl: i64) -> Self {
        Self {
            now,
            ttl,
            anchor: "",
        }
    }

    pub(super) fn anchored(self, anchor: &'a str) -> Self {
        Self { anchor, ..self }
    }
}

/// Claims `path` for the `(room, owner)` seat, which the caller expects to succeed.
pub(super) fn take<S: Store>(s: &S, seat: (&str, &str), path: &str, lease: Lease) {
    try_take(s, seat, path, lease).unwrap();
}

pub(super) fn try_take<S: Store>(
    s: &S,
    seat: (&str, &str),
    path: &str,
    lease: Lease,
) -> Result<()> {
    let request = ClaimRequest::new(seat, path, lease.ttl, "").anchored(lease.anchor);
    s.claim(&request, lease.now)
}

/// Whether a claim the store refused was refused as a deadlock (None when it was not refused).
pub(super) fn refused_as_deadlock<S: Store>(
    s: &S,
    seat: (&str, &str),
    path: &str,
    lease: Lease,
) -> Option<bool> {
    deadlocked(try_take(s, seat, path, lease))
}

pub(super) type Pairs = Vec<(String, String)>;

pub(super) fn changes<S: Store>(s: &S, room: &str, member: &str, now: i64) -> (Pairs, Pairs) {
    let c = s.claim_changes(room, member, now).unwrap();
    (
        c.added.into_iter().map(|c| (c.owner, c.path)).collect(),
        c.released.into_iter().map(|c| (c.owner, c.path)).collect(),
    )
}

pub(super) fn pair(o: &str, p: &str) -> (String, String) {
    (o.to_string(), p.to_string())
}

pub mod anchors;
pub mod asks;
pub mod claims;
pub mod credentials;
pub mod members;
pub mod messages;
pub mod policy;
