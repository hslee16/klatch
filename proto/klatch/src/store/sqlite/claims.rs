//! Taking a claim: the conflict check, the record of who is waiting for whom (so a conflict that closes a cycle is
//! recognised as a deadlock), and the short lease an `ask:<N>` claim carries.

use super::{SqliteStore, WAIT_WINDOW_SECS};
use crate::domain::{AgedClaim, Error, Result, due_milestone, overlaps, scopes_collide};
use crate::store::ClaimRequest;
use rusqlite::params;

/// Everything one attempt to take a claim needs: the request and the moment.
struct Attempt<'a> {
    room: &'a str,
    owner: &'a str,
    scope: &'a str,
    path: &'a str,
    ttl: i64,
    reason: &'a str,
    anchor: &'a str,
    now: i64,
}

/// A live claim.
struct Held {
    path: String,
    owner: String,
    expires: i64,
    scope: String,
    /// When it was first held; 0 when unknown (#547).
    since: i64,
}

/// A member's refused claim attempt, remembered as a wait.
struct Wait {
    member: String,
    path: String,
    scope: String,
}

/// The members `node` is waiting for: holders of a claim that overlaps something `node` waits for, in a scope
/// that collides with the wait.
fn blockers<'a>(node: &str, waits: &[Wait], held: &'a [Held]) -> Vec<&'a str> {
    let mut out = Vec::new();
    for w in waits.iter().filter(|w| w.member == node) {
        let blocks = |h: &&Held| {
            h.owner != w.member && overlaps(&h.path, &w.path) && scopes_collide(&h.scope, &w.scope)
        };
        out.extend(held.iter().filter(blocks).map(|h| h.owner.as_str()));
    }
    out
}

/// Whether `target` can be reached from `start` by following who waits for whom.
fn reaches(start: &str, target: &str, waits: &[Wait], held: &[Held]) -> bool {
    let mut seen: Vec<&str> = Vec::new();
    let mut stack = vec![start];
    while let Some(node) = stack.pop() {
        if node == target {
            return true;
        }
        if !seen.contains(&node) {
            seen.push(node);
            stack.extend(blockers(node, waits, held));
        }
    }
    false
}

/// Whether the live claim `h` refuses the attempt: another member's, overlapping, in a colliding scope.
fn refuses(h: &Held, req: &Attempt) -> bool {
    h.owner != req.owner && overlaps(&h.path, req.path) && scopes_collide(&h.scope, req.scope)
}

/// Seconds a claim has been held, 0 when its start is unknown.
fn held_for(since: i64, now: i64) -> i64 {
    if since > 0 { (now - since).max(0) } else { 0 }
}

/// The ask number in an `ask:<N>` claim path.
fn ask_number(path: &str) -> Option<i64> {
    path.strip_prefix("ask:").and_then(|n| n.parse().ok())
}

impl SqliteStore {
    fn live_claims(&self, room: &str) -> Result<Vec<Held>> {
        self.rows(
            "SELECT path,owner,expires,scope,since FROM claims WHERE room=?1",
            params![room],
            |r| {
                Ok(Held {
                    path: r.get(0)?,
                    owner: r.get(1)?,
                    expires: r.get(2)?,
                    scope: r.get(3)?,
                    since: r.get(4)?,
                })
            },
        )
    }

    /// Whether a refusal of `req` closes a cycle of waits: some holder that refuses it is waiting (a refused claim
    /// attempt within the window) for something a member holds who is waiting for something ... that the requester
    /// holds. Every holder that refuses counts, not only the one named: a cycle through the second of two holders is
    /// as much a deadlock (#648, found by the model).
    fn closes_cycle(&self, req: &Attempt, held: &[Held]) -> Result<bool> {
        let waits = self.room_waits(req.room)?;
        Ok(held
            .iter()
            .filter(|h| refuses(h, req))
            .any(|h| reaches(&h.owner, req.owner, &waits, held)))
    }

    fn room_waits(&self, room: &str) -> Result<Vec<Wait>> {
        self.rows(
            "SELECT member,path,scope FROM claim_waits WHERE room=?1",
            params![room],
            |r| {
                Ok(Wait {
                    member: r.get(0)?,
                    path: r.get(1)?,
                    scope: r.get(2)?,
                })
            },
        )
    }

    /// A claim just taken is a new hold edge: every waiter it now blocks waits for the new holder. When the holder
    /// waits, directly or through others, for something that waiter holds, the claim completed a cycle that no
    /// refusal closed, and the waiter is marked to be told (#648). One waiter per cycle: the cycle has one edge
    /// into the new holder.
    fn tell_completed_cycles(&self, req: &Attempt) -> Result<()> {
        let waits = self.room_waits(req.room)?;
        let held = self.live_claims(req.room)?;
        for w in waits.iter().filter(|w| {
            w.member != req.owner
                && overlaps(&w.path, req.path)
                && scopes_collide(&w.scope, req.scope)
                && reaches(req.owner, &w.member, &waits, &held)
        }) {
            self.exec(
                "UPDATE claim_waits SET told=1 WHERE room=?1 AND member=?2 AND path=?3",
                params![req.room, w.member, w.path],
            )?;
        }
        Ok(())
    }

    /// Remember that the requester is waiting for the path, and say what refused it.
    fn refuse(&self, req: &Attempt, blocker: &Held, held: &[Held]) -> Result<Error> {
        self.exec(
            "INSERT INTO claim_waits(room,member,path,ts,scope) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(room,member,path) DO UPDATE SET ts=?4, scope=?5, told=0",
            params![req.room, req.owner, req.path, req.now, req.scope],
        )?;
        Ok(Error::Conflict {
            path: blocker.path.clone(),
            owner: blocker.owner.clone(),
            secs_left: crate::domain::secs_between(blocker.expires, req.now),
            held_secs: held_for(blocker.since, req.now),
            deadlock: self.closes_cycle(req, held)?,
        })
    }

    /// Taking the path ends this member's wait for it (and for anything it overlaps).
    fn end_waits(&self, req: &Attempt) -> Result<()> {
        let mine: Vec<String> = self.rows(
            "SELECT path FROM claim_waits WHERE room=?1 AND member=?2",
            params![req.room, req.owner],
            |r| r.get(0),
        )?;
        for w in mine.into_iter().filter(|w| overlaps(w, req.path)) {
            self.exec(
                "DELETE FROM claim_waits WHERE room=?1 AND member=?2 AND path=?3",
                params![req.room, req.owner, w],
            )?;
        }
        Ok(())
    }

    /// When the owner's live claim on this path began and the milestone it was last told about: a claim taken again
    /// while it is still held keeps its age (expired rows were deleted by `attempt`, so a lapse starts afresh).
    fn prior_age(&self, req: &Attempt) -> Result<(i64, i64)> {
        let prior = self.optional(
            "SELECT since,age_noted FROM claims WHERE room=?1 AND owner=?2 AND path=?3",
            params![req.room, req.owner, req.path],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )?;
        Ok(match prior {
            Some((since, noted)) if since > 0 => (since, noted),
            _ => (req.now, 0),
        })
    }

    fn insert_claim(&self, req: &Attempt) -> Result<()> {
        let (since, noted) = self.prior_age(req)?;
        self.exec(
            "DELETE FROM claims WHERE room=?1 AND owner=?2 AND path=?3",
            params![req.room, req.owner, req.path],
        )?;
        // a note this owner left when it released the path is about that earlier claim: if this one lapses instead of
        // being released, the note must not be shown as if it were about this one (#315). A note left by another
        // owner stays for the next claimant.
        self.exec(
            "DELETE FROM claim_notes WHERE room=?1 AND path=?2 AND owner=?3",
            params![req.room, req.path, req.owner],
        )?;
        self.exec(
            "INSERT INTO claims(room,path,owner,member,reason,expires,scope,anchor,ttl,since,age_noted) VALUES(?1,?2,?3,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![req.room, req.path, req.owner, req.reason, req.now + req.ttl, req.scope, req.anchor, req.ttl, since, noted],
        )?;
        if let Some(seq) = ask_number(req.path) {
            self.exec(
                "INSERT INTO ask_leases(room,seq,owner,expires) VALUES(?1,?2,?3,?4)
                 ON CONFLICT(room,seq) DO UPDATE SET owner=?3, expires=?4",
                params![req.room, seq, req.owner, req.now + req.ttl],
            )?;
        }
        Ok(())
    }

    /// Take the claim or say what refused it. The outcome is a value, not an error, so that a refused attempt is
    /// still committed: it is remembered as a wait, which is what lets a later conflict be recognised as closing a
    /// cycle.
    fn attempt(&self, req: &Attempt) -> Result<std::result::Result<(), Error>> {
        self.exec("DELETE FROM claims WHERE expires<=?1", params![req.now])?;
        self.exec(
            "DELETE FROM claim_waits WHERE ts<?1",
            params![req.now - WAIT_WINDOW_SECS],
        )?;
        let held = self.live_claims(req.room)?;
        let blocker = held.iter().find(|h| refuses(h, req));
        if let Some(blocker) = blocker {
            return Ok(Err(self.refuse(req, blocker, &held)?));
        }
        self.end_waits(req)?;
        self.insert_claim(req)?;
        self.tell_completed_cycles(req)?;
        Ok(Ok(()))
    }

    pub(super) fn take_claim(&self, request: &ClaimRequest, now: i64) -> Result<()> {
        let attempt = Attempt {
            room: request.room,
            owner: request.owner,
            scope: request.scope,
            path: request.path,
            ttl: request.ttl,
            reason: request.reason,
            anchor: request.anchor,
            now,
        };
        self.immediate(|| self.attempt(&attempt))?
    }
}

impl SqliteStore {
    /// The member's waits marked by `tell_completed_cycles` that are still within the window and still on a cycle,
    /// each told once: the marks are cleared whether or not they are reported.
    pub(super) fn cycle_notices(
        &self,
        room: &str,
        member: &str,
        now: i64,
    ) -> Result<Vec<crate::domain::CycleNotice>> {
        self.immediate(|| {
            let marked: Vec<(String, String)> = self.rows(
                "SELECT path,scope FROM claim_waits WHERE room=?1 AND member=?2 AND told=1 AND ts>=?3 ORDER BY path",
                params![room, member, now - WAIT_WINDOW_SECS],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            self.exec(
                "UPDATE claim_waits SET told=0 WHERE room=?1 AND member=?2 AND told=1",
                params![room, member],
            )?;
            let waits = self.room_waits(room)?;
            let mut held = self.live_claims(room)?;
            held.retain(|h| h.expires > now);
            let mut out = Vec::new();
            for (path, scope) in marked {
                // a holder that blocks the wait and waits, through others, for something this member holds
                let holder = held.iter().find(|h| {
                    h.owner != member
                        && overlaps(&h.path, &path)
                        && scopes_collide(&h.scope, &scope)
                        && reaches(&h.owner, member, &waits, &held)
                });
                if let Some(h) = holder {
                    out.push(crate::domain::CycleNotice {
                        path,
                        holder: h.owner.clone(),
                    });
                }
            }
            Ok(out)
        })
    }

    /// The owner's live claims in the room that passed an age milestone since it was last told; marks each as told.
    /// Asks are left out: their leases are capped and have their own reminders.
    pub(super) fn aged_claims(&self, room: &str, owner: &str, now: i64) -> Result<Vec<AgedClaim>> {
        self.immediate(|| {
            let rows: Vec<(String, i64, i64)> = self.rows(
                "SELECT path,since,age_noted FROM claims WHERE room=?1 AND owner=?2 AND expires>?3
                     AND since>0 AND path NOT LIKE 'ask:%' ORDER BY path",
                params![room, owner, now],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            let mut aged = Vec::new();
            for (path, since, noted) in rows {
                let Some(milestone) = due_milestone(since, noted, now) else {
                    continue;
                };
                self.exec(
                    "UPDATE claims SET age_noted=?4 WHERE room=?1 AND owner=?2 AND path=?3",
                    params![room, owner, path, milestone],
                )?;
                aged.push(AgedClaim {
                    path,
                    held_secs: held_for(since, now),
                });
            }
            Ok(aged)
        })
    }
}
