//! The `ClaimStore` area of the store seam; `Store` in the parent module combines every area.

use crate::domain::{Claim, ClaimChanges, ReleaseNote, Result};
use crate::store::{ClaimRequest, Release};

pub trait ClaimStore {
    /// Take an advisory claim; refuse with `Error::Conflict` if another member holds an overlapping one.
    /// Expired claims are purged first. Re-claiming your own path replaces it. A claim conflicts only with overlapping
    /// claims in the same scope or in the shared scope `*`, and a shared claim conflicts with every scope (DESIGN
    /// section 27).
    fn claim(&self, request: &ClaimRequest, now: i64) -> Result<()>;

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

    /// Release every live claim anchored to `anchor` (the work item was reported done) and clear the lapsed ones;
    /// returns `(room, path, owner)` of each live claim released. The default releases nothing.
    fn release_anchored(&self, _anchor: &str, _now: i64) -> Result<Vec<(String, String, String)>> {
        Ok(Vec::new())
    }

    /// The distinct anchors of live claims, sorted: what a poll has to ask about.
    fn anchors_in_use(&self, _now: i64) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    /// Release every claim `member` holds in every room, as when its session ends (#277): a member that has ended
    /// cannot be working. An `ask:<N>` claim ends as a lapse, so the asker is told that the helper's claim ended
    /// without an answer. Returns how many unexpired claims were released. The default releases nothing.
    fn release_member_claims(&self, _member: &str, _now: i64) -> Result<usize> {
        Ok(0)
    }

    /// Every member that holds a claim row, expired or not (an expired row stays until a claim attempt deletes it),
    /// ordered by name. For the lease rules (#304). The default knows none.
    fn claim_owners(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    /// Move the expiry of `member`'s claims to `until`: later when `extend`, earlier otherwise, never the other
    /// way. `ask:<N>` claims keep their own lease. Returns how many rows changed. The default changes nothing.
    fn retime_member_claims(&self, _member: &str, _until: i64, _extend: bool) -> Result<usize> {
        Ok(0)
    }

    /// Unexpired claims in a room, ordered by path.
    fn claims(&self, room: &str, now: i64) -> Result<Vec<Claim>>;

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

    /// The member's own live claims in a room that passed an age milestone (2 h, 8 h, then daily) since it was last
    /// told, each reported once (#547). The default reports nothing.
    fn claim_ages(
        &self,
        _room: &str,
        _owner: &str,
        _now: i64,
    ) -> Result<Vec<crate::domain::AgedClaim>> {
        Ok(Vec::new())
    }

    /// The member's own refused claims that a claim being taken by someone else has turned into a cycle of waits,
    /// each reported once (#648). The default reports nothing.
    fn deadlock_notices(
        &self,
        _room: &str,
        _member: &str,
        _now: i64,
    ) -> Result<Vec<crate::domain::CycleNotice>> {
        Ok(Vec::new())
    }
}
