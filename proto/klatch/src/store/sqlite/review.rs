//! The review queue (#48): a table of pointers to messages that wait for a judge's verdict.

use super::{SqliteStore, be};
use crate::domain::Result;
use crate::store::{NewReview, QueuedReview, ReviewResolution};
use rusqlite::params;

impl SqliteStore {
    /// Queue `item`, or return the id already waiting for it. With a `cap`, the requester's waiting crossings are
    /// counted in the same immediate transaction as the insert, and `None` means the cap stopped it (#1736).
    pub(super) fn enqueue_review_row(
        &self,
        item: &NewReview,
        now: i64,
        cap: Option<usize>,
    ) -> Result<Option<i64>> {
        self.immediate(|| {
            // Only the requester's own item for the same target room counts as "already waiting", capped or not: another
            // member's item is never handed back, so a member cannot learn that someone else queued a message, cannot be
            // told "queued" for a request never made, and a verdict never lands on another member's target (#1736,
            // #1743). A plain item has no requester and no target room, so it still dedupes by message and label.
            let waiting = self.optional(
                "SELECT id FROM review_queue WHERE room=?1 AND seq=?2 AND to_label=?3 AND resolved IS NULL AND requester=?4 AND to_room=?5",
                params![item.room, item.seq, item.to_label, item.requester, item.to_room],
                |r| r.get(0),
            )?;
            if let Some(id) = waiting {
                return Ok(Some(id));
            }
            if let Some(cap) = cap {
                let n: i64 = self
                    .optional(
                        "SELECT COUNT(*) FROM review_queue WHERE resolved IS NULL AND to_room<>'' AND requester=?1",
                        params![item.requester],
                        |r| r.get(0),
                    )?
                    .unwrap_or(0);
                if usize::try_from(n).unwrap_or(usize::MAX) >= cap {
                    return Ok(None);
                }
            }
            self.exec(
                "INSERT INTO review_queue(room,seq,to_label,reason,ts,to_room,requester) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    item.room,
                    item.seq,
                    item.to_label,
                    item.reason,
                    now,
                    item.to_room,
                    item.requester
                ],
            )?;
            Ok(Some(self.conn.last_insert_rowid()))
        })
    }

    pub(super) fn pending_review_rows(&self, limit: usize) -> Result<Vec<QueuedReview>> {
        self.review_rows("resolved IS NULL ORDER BY id", limit)
    }

    pub(super) fn pending_review_rows_after(
        &self,
        after: i64,
        limit: usize,
    ) -> Result<Vec<QueuedReview>> {
        // `after` is an integer, so formatting it in cannot carry input into the statement
        self.review_rows(
            &format!("resolved IS NULL AND id>{} ORDER BY id", after),
            limit,
        )
    }

    pub(super) fn waiting_crossing_count(&self, requester: &str) -> Result<usize> {
        let n: i64 = self
            .optional(
                "SELECT COUNT(*) FROM review_queue WHERE resolved IS NULL AND to_room<>'' AND requester=?1",
                params![requester],
                |r| r.get(0),
            )?
            .unwrap_or(0);
        Ok(usize::try_from(n).unwrap_or(0))
    }

    /// Rows matching a fixed `WHERE ... ORDER BY` tail (never built from input).
    pub(super) fn review_rows(&self, tail: &str, limit: usize) -> Result<Vec<QueuedReview>> {
        let sql = format!(
            "SELECT id,room,seq,to_label,reason,ts,to_room,requester,COALESCE(verdict,''),COALESCE(detail,''),redacted,applied \
             FROM review_queue WHERE {tail} LIMIT ?1"
        );
        self.rows(&sql, params![limit as i64], |r| {
            Ok(QueuedReview {
                id: r.get(0)?,
                room: r.get(1)?,
                seq: r.get(2)?,
                to_label: r.get(3)?,
                reason: r.get(4)?,
                ts: r.get(5)?,
                to_room: r.get(6)?,
                requester: r.get(7)?,
                verdict: r.get(8)?,
                detail: r.get(9)?,
                redacted: r.get(10)?,
                applied: r.get(11)?,
            })
        })
    }

    pub(super) fn set_review_flag_row(&self, id: i64, note: &str) -> Result<()> {
        self.exec(
            "UPDATE review_queue SET detail=?2 WHERE id=?1 AND resolved IS NULL AND COALESCE(detail,'') <> ?2",
            params![id, note],
        )
        .map(|_| ())
    }

    pub(super) fn set_review_text_row(&self, id: i64, text: &str) -> Result<()> {
        self.exec(
            "UPDATE review_queue SET redacted=?2 WHERE id=?1 AND resolved IS NULL",
            params![id, text],
        )
        .map(|_| ())
    }

    pub(super) fn mark_review_applied_row(&self, id: i64, outcome: &str) -> Result<()> {
        self.exec(
            "UPDATE review_queue SET applied=?2 WHERE id=?1",
            params![id, outcome],
        )
        .map(|_| ())
    }

    pub(super) fn resolve_review_row(
        &self,
        resolution: &ReviewResolution,
        now: i64,
    ) -> Result<bool> {
        let ReviewResolution {
            id,
            verdict,
            detail,
        } = *resolution;
        let changed = self
            .conn
            .execute(
                "UPDATE review_queue SET verdict=?2, detail=?3, resolved=?4 WHERE id=?1 AND resolved IS NULL",
                params![id, verdict, detail, now],
            )
            .map_err(be)?;
        Ok(changed == 1)
    }

    /// The member's crossings from `room` with news not yet told (#679). Stage 1 is `waiting` (no verdict, or a
    /// verdict not applied yet), stage 2 is final (denied, or applied as released or blocked); a row is returned
    /// when its stage is past what was told, and `told` then moves up to that stage.
    pub(super) fn crossing_notice_rows(
        &self,
        room: &str,
        member: &str,
    ) -> Result<Vec<QueuedReview>> {
        self.immediate(|| {
            let rows = self.rows(
                "SELECT id,room,seq,to_label,reason,ts,to_room,requester,COALESCE(verdict,''),COALESCE(detail,''),redacted,applied,told \
                 FROM review_queue WHERE room=?1 AND requester=?2 AND told<2 AND to_room<>'' ORDER BY id",
                params![room, member],
                |r| {
                    let q = QueuedReview {
                        id: r.get(0)?,
                        room: r.get(1)?,
                        seq: r.get(2)?,
                        to_label: r.get(3)?,
                        reason: r.get(4)?,
                        ts: r.get(5)?,
                        to_room: r.get(6)?,
                        requester: r.get(7)?,
                        verdict: r.get(8)?,
                        detail: r.get(9)?,
                        redacted: r.get(10)?,
                        applied: r.get(11)?,
                    };
                    Ok((q, r.get::<_, i64>(12)?))
                },
            )?;
            let mut out = Vec::new();
            for (q, told) in rows {
                let stage = if q.verdict == "deny" || !q.applied.is_empty() {
                    2
                } else {
                    1
                };
                if stage > told {
                    self.exec(
                        "UPDATE review_queue SET told=?2 WHERE id=?1",
                        params![q.id, stage],
                    )?;
                    out.push(q);
                }
            }
            Ok(out)
        })
    }
}
