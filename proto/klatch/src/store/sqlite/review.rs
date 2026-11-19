//! The review queue (#48): a table of pointers to messages that wait for a judge's verdict.

use super::{SqliteStore, be};
use crate::domain::Result;
use crate::store::{NewReview, QueuedReview, ReviewResolution};
use rusqlite::params;

impl SqliteStore {
    pub(super) fn enqueue_review_row(&self, item: &NewReview, now: i64) -> Result<i64> {
        self.immediate(|| {
            let waiting = self.optional(
                "SELECT id FROM review_queue WHERE room=?1 AND seq=?2 AND to_label=?3 AND resolved IS NULL",
                params![item.room, item.seq, item.to_label],
                |r| r.get(0),
            )?;
            if let Some(id) = waiting {
                return Ok(id);
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
            Ok(self.conn.last_insert_rowid())
        })
    }

    pub(super) fn pending_review_rows(&self, limit: usize) -> Result<Vec<QueuedReview>> {
        self.review_rows("resolved IS NULL ORDER BY id", limit)
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
