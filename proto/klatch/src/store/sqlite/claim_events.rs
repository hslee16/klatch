//! The record of claim events (#1386): counts and class only, never a claim name, a member, a reason or a note.
//!
//! One small row per event in `claim_events`: the moment, the room, an event word (`granted`, `refused`, `released`,
//! `expired`, `takeover`), the claim class (`file`, `ticket`, `ask`, `other`, from the name prefix alone), a reason
//! word from a fixed vocabulary, the seconds the claim was held and, for a grant that ended a refusal, the seconds the
//! claimant waited. Recording is a side effect of the existing claim paths and changes none of them: a recording
//! failure is swallowed and counted (`claim_record_errors`), never returned, and every statement runs inside the
//! transaction the claim operation already holds, so no lock is held longer.
//!
//! Retention: a row older than `CLAIM_EVENT_MAX_AGE_SECS` is deleted and the table keeps at most
//! `CLAIM_EVENT_MAX_ROWS` rows (the newest), both applied as each event is written. A renewal of a claim the holder
//! already has is not an event.

use super::SqliteStore;
use crate::domain::{CLAIM_MAX_SECS, Result, claim_class};
use rusqlite::params;

/// The key of one claim: where it is, who holds it, and the path.
#[derive(Clone, Copy)]
pub(super) struct ClaimKey<'a> {
    pub(super) room: &'a str,
    pub(super) owner: &'a str,
    pub(super) path: &'a str,
}

impl<'a> ClaimKey<'a> {
    pub(super) fn new(room: &'a str, owner: &'a str, path: &'a str) -> Self {
        Self { room, owner, path }
    }
}

/// Rows older than this are deleted as events are written: 30 days.
pub const CLAIM_EVENT_MAX_AGE_SECS: i64 = 30 * 86_400;

/// The table keeps at most this many rows, the newest.
pub const CLAIM_EVENT_MAX_ROWS: i64 = 20_000;

/// The passive claim events (#1401, `passive_granted` and the like) have a cap of their own, so the many renewals of
/// busy editing never push the explicit claim history out of the table.
pub const PASSIVE_EVENT_MAX_ROWS: i64 = 20_000;

/// One claim event to record.
pub(super) struct ClaimEvent<'a> {
    pub room: &'a str,
    /// The claim name, used only to find its class.
    pub path: &'a str,
    pub event: &'static str,
    pub reason: &'static str,
    pub held: i64,
    pub waited: Option<i64>,
}

impl SqliteStore {
    /// How many claim events could not be recorded since this store was opened.
    pub fn claim_record_errors(&self) -> u64 {
        self.claim_record_errors.get()
    }

    /// Record an event; a failure is counted and swallowed.
    pub(super) fn record_claim_event(&self, now: i64, e: &ClaimEvent) {
        if self.try_record(now, e).is_err() {
            self.claim_record_errors
                .set(self.claim_record_errors.get() + 1);
        }
    }

    fn try_record(&self, now: i64, e: &ClaimEvent) -> Result<()> {
        self.exec(
            "INSERT INTO claim_events(ts,room,event,class,reason,held,waited) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![now, e.room, e.event, claim_class(e.path), e.reason, e.held, e.waited],
        )?;
        self.exec(
            "DELETE FROM claim_events WHERE ts<?1",
            params![now - CLAIM_EVENT_MAX_AGE_SECS],
        )?;
        // each kind keeps its own newest rows: the explicit events and the passive ones never evict each other
        let (like, cap) = if e.event.starts_with("passive_") {
            ("LIKE", PASSIVE_EVENT_MAX_ROWS)
        } else {
            ("NOT LIKE", CLAIM_EVENT_MAX_ROWS)
        };
        self.exec(
            &format!(
                "DELETE FROM claim_events WHERE event {like} 'passive\\_%' ESCAPE '\\' AND id <= \
                 COALESCE((SELECT id FROM claim_events WHERE event {like} 'passive\\_%' ESCAPE '\\' \
                 ORDER BY id DESC LIMIT 1 OFFSET ?1), 0)"
            ),
            params![cap],
        )?;
        Ok(())
    }

    /// Remember that `member` was refused `path` (the first refusal keeps its time), so the grant that ends the
    /// conflict can say how long it took.
    pub(super) fn open_claim_conflict(&self, key: ClaimKey<'_>, now: i64) {
        let ClaimKey {
            room,
            owner: member,
            path,
        } = key;
        let r = self
            .exec(
                "INSERT INTO claim_conflicts_open(room,member,path,first_ts) VALUES(?1,?2,?3,?4)
                 ON CONFLICT(room,member,path) DO NOTHING",
                params![room, member, path, now],
            )
            .and_then(|_| {
                self.exec(
                    "DELETE FROM claim_conflicts_open WHERE first_ts<?1",
                    params![now - CLAIM_MAX_SECS],
                )
            });
        if r.is_err() {
            self.claim_record_errors
                .set(self.claim_record_errors.get() + 1);
        }
    }

    /// The seconds since the first refusal of `member` for a claim overlapping `path`, if any; the refusals are
    /// closed.
    pub(super) fn close_claim_conflicts(&self, key: ClaimKey<'_>, now: i64) -> Option<i64> {
        let ClaimKey {
            room,
            owner: member,
            path,
        } = key;
        let r = (|| -> Result<Option<i64>> {
            let open: Vec<(String, i64)> = self.rows(
                "SELECT path,first_ts FROM claim_conflicts_open WHERE room=?1 AND member=?2",
                params![room, member],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let mut first: Option<i64> = None;
            for (p, ts) in open
                .into_iter()
                .filter(|(p, _)| crate::domain::overlaps(p, path))
            {
                first = Some(first.map_or(ts, |f| f.min(ts)));
                self.exec(
                    "DELETE FROM claim_conflicts_open WHERE room=?1 AND member=?2 AND path=?3",
                    params![room, member, p],
                )?;
            }
            Ok(first.map(|f| (now - f).max(0)))
        })();
        r.unwrap_or_else(|_| {
            self.claim_record_errors
                .set(self.claim_record_errors.get() + 1);
            None
        })
    }
}
