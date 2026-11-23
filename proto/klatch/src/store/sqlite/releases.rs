//! Releasing claims, singly, by anchor and for a whole member, and what a member is told changed.

use super::claim_events::{ClaimEvent, ClaimKey};
use super::claims::held_for;
use super::*;
use crate::store::ClaimStore;

/// A claim that just ended: where, when it was taken and due, and the moment the caller knows (`None` when it has no
/// clock).
pub(super) struct Ended<'a> {
    pub(super) room: &'a str,
    pub(super) path: &'a str,
    pub(super) since: i64,
    pub(super) expires: i64,
    pub(super) now: Option<i64>,
}

impl<'a> Ended<'a> {
    /// A claim that ended at `now` (an `i64`, or an `Option` when unknown); `(since, expires)` are the stored times.
    pub(super) fn at(
        room: &'a str,
        path: &'a str,
        (since, expires): (i64, i64),
        now: impl Into<Option<i64>>,
    ) -> Self {
        Self {
            room,
            path,
            since,
            expires,
            now: now.into(),
        }
    }
}

impl SqliteStore {
    pub(super) fn do_release_anchored(
        &self,
        anchor: &str,
        now: i64,
    ) -> Result<Vec<(String, String, String)>> {
        if anchor.is_empty() {
            return Ok(Vec::new());
        }
        self.immediate(|| {
            let held: Vec<(String, String, String, i64, i64)> = self.rows(
                "SELECT room,path,owner,expires,since FROM claims WHERE anchor=?1 ORDER BY room,path",
                params![anchor],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )?;
            let mut released = Vec::new();
            for (room, path, owner, expires, since) in held {
                self.exec(
                    "DELETE FROM claims WHERE room=?1 AND path=?2 AND owner=?3",
                    params![room, path, owner],
                )?;
                self.record_ended(Ended::at(&room, &path, (since, expires), now), "anchor");
                if expires > now {
                    released.push((room, path, owner));
                }
            }
            Ok(released)
        })
    }

    /// Record that a claim ended (#1386): released by `way` while its lease ran, or an expiry when the lease had lapsed
    /// before anything removed the row. `now` is `None` when the caller has no clock: the hold time is then unknown (0) and the event is stamped with the
    /// start of the claim.
    pub(super) fn record_ended(&self, ended: Ended<'_>, way: &'static str) {
        let Ended {
            room,
            path,
            since,
            expires,
            now,
        } = ended;
        let (event, reason, held) = match now {
            Some(now) if expires <= now => ("expired", "", held_for(since, expires)),
            Some(now) => ("released", way, held_for(since, now)),
            None => ("released", way, 0),
        };
        self.record_claim_event(
            now.unwrap_or(since.max(0)),
            &ClaimEvent {
                room,
                path,
                event,
                reason,
                held,
                waited: None,
            },
        );
    }

    /// Release a claim; `now` is the moment when the caller knows it (the hold time is recorded from it).
    pub(super) fn do_release(&self, key: ClaimKey<'_>, now: Option<i64>) -> Result<()> {
        let ClaimKey { room, owner, path } = key;
        let before: Option<(i64, i64)> = self
            .optional(
                "SELECT since,expires FROM claims WHERE room=?1 AND path=?2 AND owner=?3",
                params![room, path, owner],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or(None);
        if let Some(seq) = path
            .strip_prefix("ask:")
            .and_then(|n| n.parse::<i64>().ok())
        {
            // A release on purpose is not a lapse.
            self.exec(
                "DELETE FROM ask_leases WHERE room=?1 AND seq=?2 AND owner=?3",
                params![room, seq, owner],
            )?;
        }
        let n = self.exec(
            "DELETE FROM claims WHERE room=?1 AND path=?2 AND owner=?3",
            params![room, path, owner],
        )?;
        if n == 0 {
            return Err(Error::NotFound(format!("no claim by {owner} on {path}")));
        }
        if let Some((since, expires)) = before {
            self.record_ended(Ended::at(room, path, (since, expires), now), "holder");
        }
        Ok(())
    }

    pub(super) fn do_release_with(&self, release: &Release, now: i64) -> Result<()> {
        let Release {
            room,
            owner,
            path,
            note,
        } = *release;
        self.immediate(|| {
            self.do_release(ClaimKey { room, owner, path }, Some(now))?;
            if note.is_empty() {
                self.exec(
                    "DELETE FROM claim_notes WHERE room=?1 AND path=?2",
                    params![room, path],
                )?;
            } else {
                self.exec(
                    "INSERT INTO claim_notes(room,path,owner,note,ts) VALUES(?1,?2,?3,?4,?5)
                         ON CONFLICT(room,path) DO UPDATE SET owner=?3, note=?4, ts=?5",
                    params![room, path, owner, note, now],
                )?;
            }
            Ok(())
        })
    }

    /// Release the passive claims of `member` in every room (#1401): one `passive_released` event for each that was
    /// still live (or `passive_expired` when its lease had lapsed), and nothing for any other claim of the member.
    pub(super) fn do_release_passive(&self, member: &str, now: i64) -> Result<usize> {
        self.immediate(|| {
            let held: Vec<(String, String, i64, i64)> = self.rows(
                "SELECT room,path,expires,since FROM claims WHERE owner=?1 AND reason=?2",
                params![member, crate::domain::PASSIVE_REASON],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            let mut released = 0;
            for (room, path, expires, since) in held {
                self.exec(
                    "DELETE FROM claims WHERE room=?1 AND path=?2 AND owner=?3",
                    params![room, path, member],
                )?;
                let live = expires > now;
                released += usize::from(live);
                self.record_claim_event(
                    now,
                    &ClaimEvent {
                        room: &room,
                        path: &path,
                        event: if live {
                            "passive_released"
                        } else {
                            "passive_expired"
                        },
                        reason: "",
                        held: held_for(since, if live { now } else { expires }),
                        waited: None,
                    },
                );
            }
            Ok(released)
        })
    }

    pub(super) fn do_release_member_claims(&self, member: &str, now: i64) -> Result<usize> {
        self.immediate(|| {
            let held: Vec<(String, String, i64, i64)> = self.rows("SELECT room,path,expires,since FROM claims WHERE owner=?1", params![member], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            let mut released = 0;
            for (room, path, expires, since) in held {
                self.record_ended(Ended::at(&room, &path, (since, expires), now), "session");
                if expires > now {
                    released += 1;
                    if let Some(seq) = path
                        .strip_prefix("ask:")
                        .and_then(|n| n.parse::<i64>().ok())
                    {
                        // ends the lease now, so the asker is told it lapsed
                        self.exec(
                                "UPDATE ask_leases SET expires=?4 WHERE room=?1 AND seq=?2 AND owner=?3 AND expires>?4",
                                params![room, seq, member, now],
                            )?;
                    }
                }
                self.exec(
                        "DELETE FROM claims WHERE room=?1 AND path=?2 AND owner=?3",
                        params![room, path, member],
                    )?;
            }
            Ok(released)
        })
    }

    pub(super) fn do_claim_changes(
        &self,
        room: &str,
        member: &str,
        now: i64,
    ) -> Result<ClaimChanges> {
        // Most digests find nothing changed; that is decided on a read snapshot, with no writer lock (#128).
        let unchanged = self.snapshot(|| {
            let (_, added, released) = self.claim_diff(room, member, now)?;
            Ok(added.is_empty() && released.is_empty())
        })?;
        if unchanged {
            return Ok(ClaimChanges::default());
        }
        self.immediate(|| {
            let (current, added, released) = self.claim_diff(room, member, now)?;
            if added.is_empty() && released.is_empty() {
                return Ok(ClaimChanges::default());
            }
            self.exec(
                "DELETE FROM claims_seen WHERE room=?1 AND member=?2",
                params![room, member],
            )?;
            for c in &current {
                self.exec(
                    "INSERT INTO claims_seen(room,member,owner,path,expires) VALUES(?1,?2,?3,?4,?5)",
                    params![room, member, c.owner, c.path, c.expires],
                )?;
            }
            Ok(ClaimChanges { added, released })
        })
    }

    /// The live claims of others, those the member has not been shown as they are, and those it was shown that are
    /// gone. A read.
    fn claim_diff(
        &self,
        room: &str,
        member: &str,
        now: i64,
    ) -> Result<(Vec<Claim>, Vec<Claim>, Vec<ReleasedClaim>)> {
        {
            let current: Vec<Claim> = self
                .claims(room, now)?
                .into_iter()
                // a passive claim (#1401) is not announced in a digest: it is read with `claims`, and met by the editor
                // it refuses; a digest line for every edited file would cost every member's context
                .filter(|c| c.owner != member && c.reason != crate::domain::PASSIVE_REASON)
                .collect();
            let mut st = self
                .conn
                .prepare("SELECT owner,path,expires FROM claims_seen WHERE room=?1 AND member=?2")
                .map_err(be)?;
            let seen: Vec<(String, String, i64)> = st
                .query_map(params![room, member], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .map_err(be)?
                .collect::<std::result::Result<_, _>>()
                .map_err(be)?;
            let added: Vec<Claim> = current
                .iter()
                .filter(|c| {
                    !seen
                        .iter()
                        .any(|(o, p, e)| *o == c.owner && *p == c.path && *e == c.expires)
                })
                .cloned()
                .collect();
            let mut released = Vec::new();
            for (o, p, _) in seen
                .iter()
                .filter(|(o, p, _)| !current.iter().any(|c| c.owner == *o && c.path == *p))
            {
                released.push(ReleasedClaim {
                    owner: o.clone(),
                    path: p.clone(),
                    note: self.note_of(room, o, p)?,
                });
            }
            Ok((current, added, released))
        }
    }
}
