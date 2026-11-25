//! The record of lapsed claims (#1586): the claim sweep writes each named lapse to `claim_lapses`, so the
//! `lapsed_claim` stuck event outlives the sweep. Bounded by the report window and a cap per room.

use super::SqliteStore;
use super::claims::ask_number;
use crate::domain::{LAPSE_RECORD_CAP, LAPSED_WINDOW_SECS, Result};
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
    /// Remember the named lapses in `claim_lapses` (#1586) so the `lapsed_claim` stuck event outlives this sweep: not
    /// passive or ask claims, not a lapse already past the report window. One row per room, path and holder (a repeat
    /// replaces it). The table is pruned past the window and, per room that gained rows, down to `LAPSE_RECORD_CAP`
    /// newest, so claiming and letting claims lapse cannot grow it without bound. Only runs when something lapsed.
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
        if lapsed.is_empty() {
            return Ok(());
        }
        self.exec("DELETE FROM claim_lapses WHERE expires<=?1", params![floor])?;
        let cap = i64::try_from(LAPSE_RECORD_CAP).unwrap_or(i64::MAX);
        for room in rooms {
            self.exec(
                "DELETE FROM claim_lapses WHERE room=?1 AND rowid NOT IN
                 (SELECT rowid FROM claim_lapses WHERE room=?1 ORDER BY expires DESC, path DESC, owner DESC LIMIT ?2)",
                params![room, cap],
            )?;
        }
        Ok(())
    }
}
