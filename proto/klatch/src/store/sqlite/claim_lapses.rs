//! The record of lapsed claims (#1586): the claim sweep writes each named lapse to `claim_lapses`, so the
//! `lapsed_claim` stuck event outlives the sweep. Bounded by the report window and a cap per room and in all.

use super::SqliteStore;
use super::claims::ask_number;
use crate::domain::{LAPSE_RECORD_CAP, LAPSE_TOTAL_CAP, LAPSED_WINDOW_SECS, Result};
use rusqlite::params;

/// A claim whose lease had lapsed and that an attempt swept away.
pub(super) struct Lapsed {
    /// It was a passive claim (#1401), so its expiry is counted apart.
    pub(super) passive: bool,
    pub(super) room: String,
    pub(super) path: String,
    pub(super) owner: String,
    pub(super) scope: String,
    /// Seconds it was held until the lease ended.
    pub(super) held: i64,
    /// When the lease ended.
    pub(super) expires: i64,
    /// When it was first held; 0 when unknown.
    pub(super) since: i64,
}

impl SqliteStore {
    /// How many lapse rows the caps dropped since this store was opened.
    pub fn claim_lapses_dropped(&self) -> u64 {
        self.claim_lapses_dropped.get()
    }

    /// Remember the named lapses in `claim_lapses` (#1586) so the `lapsed_claim` stuck event outlives this sweep: not
    /// passive or ask claims, not a lapse already past the report window. One row per room, path and holder (a repeat
    /// replaces it). The table is pruned past the window at every sweep (so the next claim attempt of any room clears old
    /// rows; a store with no attempts at all runs nothing, and the report filters by the window anyway), and, when rows
    /// were added, per room that gained rows down to `LAPSE_RECORD_CAP` newest and in all down to `LAPSE_TOTAL_CAP` (the rooms with the most rows give up theirs first), the rows dropped
    /// counted in `claim_lapses_dropped`, so
    /// claiming and letting claims lapse cannot grow it without bound.
    pub(super) fn record_lapses(&self, lapsed: &[Lapsed], now: i64) -> Result<()> {
        let floor = now - LAPSED_WINDOW_SECS;
        let named = |l: &&Lapsed| !l.passive && ask_number(&l.path).is_none() && l.expires > floor;
        let mut rooms: Vec<&str> = Vec::new();
        for l in lapsed.iter().filter(named) {
            self.exec(
                "INSERT INTO claim_lapses(room,path,owner,expires,since) VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(room,path,owner) DO UPDATE SET expires=?4, since=?5",
                params![l.room, l.path, l.owner, l.expires, l.since],
            )?;
            if !rooms.contains(&l.room.as_str()) {
                rooms.push(&l.room);
            }
        }
        // past the window on every sweep, a lapse or not: the next claim attempt of any room clears the old rows
        self.exec("DELETE FROM claim_lapses WHERE expires<=?1", params![floor])?;
        if rooms.is_empty() {
            return Ok(());
        }
        let cap = i64::try_from(LAPSE_RECORD_CAP).unwrap_or(i64::MAX);
        let mut dropped = 0;
        for room in rooms {
            dropped += self.exec(
                "DELETE FROM claim_lapses WHERE room=?1 AND rowid NOT IN
                 (SELECT rowid FROM claim_lapses WHERE room=?1 ORDER BY expires DESC, path DESC, owner DESC LIMIT ?2)",
                params![room, cap],
            )?;
        }
        // and in all, whatever the number of rooms. Fair between rooms (#1702): each room's rows are ranked newest
        // first and the deepest ranks go first (the oldest among equals), so the biggest rooms give up rows before a
        // room with a few; by age alone, ten rooms at the per-room cap would push out a quiet room's recent lapse.
        dropped += self.exec(
            "DELETE FROM claim_lapses WHERE rowid IN
             (SELECT rowid FROM
                (SELECT rowid, room, path, owner, expires,
                        ROW_NUMBER() OVER (PARTITION BY room ORDER BY expires DESC, path DESC, owner DESC) AS depth
                 FROM claim_lapses)
              ORDER BY depth DESC, expires ASC, room ASC, path ASC, owner ASC
              LIMIT MAX((SELECT COUNT(*) FROM claim_lapses) - ?1, 0))",
            params![i64::try_from(LAPSE_TOTAL_CAP).unwrap_or(i64::MAX)],
        )?;
        self.claim_lapses_dropped
            .set(self.claim_lapses_dropped.get() + dropped as u64);
        Ok(())
    }
}
