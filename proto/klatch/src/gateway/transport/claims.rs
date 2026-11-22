//! Claims on paths: take, release, notes, leases and anchors.

use super::Direct;
use crate::domain::Claim;
use crate::domain::Result;
use crate::gateway::ClaimSpec;
use crate::gateway::SharedTransport;
use crate::gateway::shared::forward;
use crate::gateway::shared::{Lend, Own};
use crate::store::Store;

/// Claims on paths: take, release, notes, leases and anchors.
pub trait Claims {
    /// Take a claim (optionally shared with every scope).
    fn claim(&self, room: &str, spec: &ClaimSpec) -> Result<()>;
    fn release(&self, room: &str, path: &str) -> Result<()>;
    /// `release` with a note on what the caller changed.
    fn release_with_note(&self, room: &str, path: &str, note: &str) -> Result<()>;
    /// The latest release of a path that carried a note.
    fn last_release(&self, room: &str, path: &str) -> Result<Option<crate::domain::ReleaseNote>>;
    /// Release every claim the caller holds in every room (its session ended); returns how many.
    fn release_all(&self) -> Result<usize>;
    /// A work item is done: release the claims anchored to it; returns how many (#746).
    fn anchor_done(&self, anchor: &str) -> Result<usize>;
    /// The lease a claim gets in a room: the one asked for, else the default for the path (the room's setting for an
    /// `ask:` claim) (#242).
    fn claim_ttl(&self, room: &str, path: &str, requested: Option<i64>) -> Result<i64>;
    fn claims(&self, room: &str) -> Result<Vec<Claim>>;
}

impl<S: Store> Claims for Direct<S> {
    fn claim(&self, room: &str, spec: &ClaimSpec) -> Result<()> {
        self.gateway.claim(&self.principal, room, spec)
    }
    fn release(&self, room: &str, path: &str) -> Result<()> {
        self.gateway.release(&self.principal, room, path)
    }
    fn release_with_note(&self, room: &str, path: &str, note: &str) -> Result<()> {
        self.gateway
            .release_with_note(&self.principal, (room, path), note)
    }
    fn last_release(&self, room: &str, path: &str) -> Result<Option<crate::domain::ReleaseNote>> {
        self.gateway.last_release(&self.principal, room, path)
    }
    fn release_all(&self) -> Result<usize> {
        self.gateway.release_all(&self.principal)
    }
    fn anchor_done(&self, anchor: &str) -> Result<usize> {
        self.gateway.anchor_done(&self.principal, anchor)
    }
    fn claim_ttl(&self, room: &str, path: &str, requested: Option<i64>) -> Result<i64> {
        self.gateway
            .claim_ttl_for(&self.principal, (room, path), requested)
    }
    fn claims(&self, room: &str) -> Result<Vec<Claim>> {
        self.gateway.claims(&self.principal, room)
    }
}

impl Claims for SharedTransport {
    forward! {
        release(room: &str, path: &str) -> Result<()>;
        release_with_note(room: &str, path: &str, note: &str) -> Result<()>;
        last_release(room: &str, path: &str) -> Result<Option<crate::domain::ReleaseNote>>;
        release_all() -> Result<usize>;
        anchor_done(anchor: &str) -> Result<usize>;
        claim_ttl(room: &str, path: &str, requested: Option<i64>) -> Result<i64>;
        claims(room: &str) -> Result<Vec<crate::domain::Claim>>;
    }

    fn claim(&self, room: &str, spec: &ClaimSpec) -> Result<()> {
        let room = Own::own(&room);
        let (path, reason, anchor) = (
            Own::own(&spec.path),
            Own::own(&spec.reason),
            Own::own(&spec.anchor),
        );
        let (ttl, shared) = (spec.ttl, spec.shared);
        self.call(move |t| {
            let mut spec = ClaimSpec::new(Lend::lend(&path), ttl, Lend::lend(&reason));
            spec.shared = shared;
            spec.anchor = Lend::lend(&anchor);
            t.claim(Lend::lend(&room), &spec)
        })
    }
}
