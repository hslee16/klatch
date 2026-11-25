//! The SQLite side of the `ClaimStore` area.

use super::super::SqliteStore;
use super::super::claim_events::ClaimKey;
use crate::domain::{Claim, ClaimChanges, PassiveTaken, ReleaseNote, Result};
use crate::store::ClaimStore;
use crate::store::{ClaimRequest, Release};
use rusqlite::params;

impl ClaimStore for SqliteStore {
    fn release_anchored(&self, anchor: &str, now: i64) -> Result<Vec<(String, String, String)>> {
        self.do_release_anchored(anchor, now)
    }

    fn anchors_in_use(&self, now: i64) -> Result<Vec<String>> {
        self.rows(
            "SELECT DISTINCT anchor FROM claims WHERE anchor<>'' AND expires>?1 ORDER BY anchor",
            params![now],
            |r| r.get(0),
        )
    }

    fn claim(&self, request: &ClaimRequest, now: i64) -> Result<()> {
        self.take_claim(request, now)
    }

    fn take_passive_claim(&self, request: &ClaimRequest, now: i64) -> Result<PassiveTaken> {
        self.do_passive_claim(request, now)
    }

    fn release_passive_claims(&self, member: &str, now: i64) -> Result<usize> {
        self.do_release_passive(member, now)
    }

    fn note_passive_failure(&self, now: i64) {
        self.record_claim_event(
            now,
            &super::super::claim_events::ClaimEvent {
                room: "",
                path: "",
                event: "passive_error",
                reason: "",
                held: 0,
                waited: None,
            },
        );
    }

    fn release(&self, room: &str, owner: &str, path: &str) -> Result<()> {
        self.do_release(ClaimKey { room, owner, path }, None)
    }

    fn release_at(&self, room: &str, owner: &str, path: &str, now: i64) -> Result<()> {
        self.do_release(ClaimKey { room, owner, path }, Some(now))
    }

    fn release_with(&self, release: &Release, now: i64) -> Result<()> {
        self.do_release_with(release, now)
    }

    fn last_release(&self, room: &str, path: &str) -> Result<Option<ReleaseNote>> {
        self.optional(
            "SELECT owner,note,ts FROM claim_notes WHERE room=?1 AND path=?2",
            params![room, path],
            |r| {
                Ok(ReleaseNote {
                    owner: r.get(0)?,
                    note: r.get(1)?,
                    ts: r.get(2)?,
                })
            },
        )
    }

    fn release_member_claims(&self, member: &str, now: i64) -> Result<usize> {
        self.do_release_member_claims(member, now)
    }

    fn claim_owners(&self) -> Result<Vec<String>> {
        self.claim_owners_all()
    }

    fn retime_member_claims(&self, member: &str, until: i64, extend: bool) -> Result<usize> {
        self.retime_claims(member, until, extend)
    }

    fn claims(&self, room: &str, now: i64) -> Result<Vec<Claim>> {
        self.rows("SELECT path,owner,reason,expires,scope FROM claims WHERE room=?1 AND expires>?2 ORDER BY path", params![room, now], |r| {
            Ok(Claim {
                path: r.get(0)?,
                owner: r.get(1)?,
                reason: r.get(2)?,
                expires: r.get(3)?,
                scope: r.get(4)?,
            })
        })
    }

    fn claim_ages(
        &self,
        room: &str,
        owner: &str,
        now: i64,
    ) -> Result<Vec<crate::domain::AgedClaim>> {
        self.aged_claims(room, owner, now)
    }

    fn wait_cycles(&self, room: &str, now: i64) -> Result<Vec<Vec<String>>> {
        self.standing_wait_cycles(room, now)
    }

    fn note_yield_release(&self, room: &str, owner: &str, path: &str, now: i64) -> Result<()> {
        self.do_note_yield_release(ClaimKey { room, owner, path }, now)
    }

    fn claim_overwrites(&self, room: &str, now: i64) -> Result<Vec<(String, String, String)>> {
        self.standing_overwrites(room, now)
    }

    fn lapsed_claims(
        &self,
        room: &str,
        now: i64,
        (window, limit): (i64, usize),
    ) -> Result<Vec<(String, String, i64)>> {
        // the swept records plus the rows nothing has swept yet; a live claim of the same holder on the same path means
        // the producer came back, so the lapse no longer stands
        self.rows(
            "SELECT path,owner,MAX(expires) FROM (
                 SELECT path,owner,expires,since FROM claim_lapses WHERE room=?1
                 UNION ALL
                 SELECT path,owner,expires,since FROM claims WHERE room=?1 AND reason<>?4 AND path NOT LIKE 'ask:%') c
             WHERE c.expires<=?2 AND c.expires>?2-?3
               AND NOT EXISTS(SELECT 1 FROM claims l WHERE l.room=?1 AND l.path=c.path AND l.owner=c.owner AND l.expires>?2)
               AND NOT EXISTS(SELECT 1 FROM members m WHERE m.room=?1 AND m.name=c.owner
                              AND m.status LIKE 'done%' AND m.status_ts>=c.since)
             GROUP BY path,owner
             ORDER BY MAX(expires) DESC, path, owner LIMIT ?5",
            params![
                room,
                now,
                window,
                crate::domain::PASSIVE_REASON,
                i64::try_from(limit).unwrap_or(i64::MAX)
            ],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
    }

    fn deadlock_notices(
        &self,
        room: &str,
        member: &str,
        now: i64,
    ) -> Result<Vec<crate::domain::CycleNotice>> {
        self.cycle_notices(room, member, now)
    }

    fn claim_changes(&self, room: &str, member: &str, now: i64) -> Result<ClaimChanges> {
        self.do_claim_changes(room, member, now)
    }
}
