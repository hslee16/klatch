//! Taking a claim: the conflict check, the record of who is waiting for whom (so a conflict that closes a cycle is
//! recognised as a deadlock), and the short lease an `ask:<N>` claim carries.

use super::claim_events::{ClaimEvent, ClaimKey};
use super::{SqliteStore, WAIT_WINDOW_SECS};
use crate::domain::{
    AgedClaim, Error, PASSIVE_REASON, PassiveTaken, Result, SHARED_SCOPE, YIELD_WINDOW_SECS,
    due_milestone, overlaps, scopes_collide,
};
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

/// A claim whose lease had lapsed and that an attempt swept away.
struct Lapsed {
    /// It was a passive claim (#1401), so its expiry is counted apart.
    passive: bool,
    room: String,
    path: String,
    owner: String,
    scope: String,
    /// Seconds it was held until the lease ended.
    held: i64,
    /// When the lease ended.
    expires: i64,
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
pub(super) fn held_for(since: i64, now: i64) -> i64 {
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

    /// The holders that refuse `req` and wait, directly or through others, for something the requester holds: the
    /// other parties to the deadlock the refusal closes, so a refusal closes a cycle when there is one (#648). Every
    /// holder that refuses counts, not only the one named: a cycle through the second of two holders is as much a
    /// deadlock (found by the model).
    fn cycle_holders(&self, req: &Attempt, held: &[Held]) -> Result<Vec<String>> {
        let waits = self.room_waits(req.room)?;
        Ok(held
            .iter()
            .filter(|h| refuses(h, req) && reaches(&h.owner, req.owner, &waits, held))
            .map(|h| h.owner.clone())
            .collect())
    }

    /// Mark the parties to a deadlock (#909): the requester and every holder on the cycle, as of now.
    fn mark_yield_parties(&self, req: &Attempt, holders: &[String]) -> Result<()> {
        for member in std::iter::once(req.owner).chain(holders.iter().map(String::as_str)) {
            self.exec(
                "INSERT INTO claim_yields(room,member,path,ts,taker) VALUES(?1,?2,'',?3,'')
                 ON CONFLICT(room,member,path) DO UPDATE SET ts=?3",
                params![req.room, member, req.now],
            )?;
        }
        Ok(())
    }

    /// A claim just taken by someone who is not the member that released an overlapping path inside the window after a
    /// deadlock: that release is now overwritten, as far as klatch can see (#909).
    fn note_overwrites(&self, req: &Attempt) -> Result<()> {
        let released: Vec<(String, String)> = self.rows(
            "SELECT member,path FROM claim_yields WHERE room=?1 AND path<>'' AND taker='' AND member<>?2 AND ts>=?3",
            params![req.room, req.owner, req.now - YIELD_WINDOW_SECS],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        for (member, path) in released.into_iter().filter(|(_, p)| overlaps(p, req.path)) {
            self.exec(
                "UPDATE claim_yields SET taker=?4, ts=?5 WHERE room=?1 AND member=?2 AND path=?3",
                params![req.room, member, path, req.owner, req.now],
            )?;
        }
        Ok(())
    }

    /// Before `owner` releases `path` (#909): when it has a live claim there and was a party to a deadlock inside the
    /// window, remember the release so a take of the path by someone else is flagged.
    pub(super) fn do_note_yield_release(&self, key: ClaimKey<'_>, now: i64) -> Result<()> {
        let ClaimKey { room, owner, path } = key;
        self.immediate(|| {
            let live = self
                .optional(
                    "SELECT 1 FROM claims WHERE room=?1 AND path=?2 AND owner=?3 AND expires>?4",
                    params![room, path, owner, now],
                    |r| r.get::<_, i64>(0),
                )?
                .is_some();
            if live {
                self.note_yield_end(room, (owner, path), now)?;
            }
            Ok(())
        })
    }

    /// A claim of `owner` on `path` ended at `at` (released, lapsed, session end or anchor release, #1224): when the
    /// owner was a party to a deadlock inside the window before `at`, remember it so a take of the path by someone else
    /// is flagged. The caller removes the claim row; this only reads the deadlock mark.
    pub(super) fn note_yield_end(&self, room: &str, held: (&str, &str), at: i64) -> Result<()> {
        let (owner, path) = held;
        let marked = self
            .optional(
                "SELECT 1 FROM claim_yields WHERE room=?1 AND member=?2 AND path='' AND ts>=?3 AND ts<=?4",
                params![room, owner, at - YIELD_WINDOW_SECS, at],
                |r| r.get::<_, i64>(0),
            )?
            .is_some();
        if marked {
            self.exec(
                "INSERT INTO claim_yields(room,member,path,ts,taker) VALUES(?1,?2,?3,?4,'')
                 ON CONFLICT(room,member,path) DO UPDATE SET ts=?4, taker=''",
                params![room, owner, path, at],
            )?;
        }
        Ok(())
    }

    /// The claims flagged by `note_overwrites` that are still inside the window: `(path, previous, new)`, sorted.
    pub(super) fn standing_overwrites(
        &self,
        room: &str,
        now: i64,
    ) -> Result<Vec<(String, String, String)>> {
        self.rows(
            "SELECT path,member,taker FROM claim_yields WHERE room=?1 AND path<>'' AND taker<>'' AND ts>=?2
             ORDER BY path,member,taker",
            params![room, now - YIELD_WINDOW_SECS],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
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
        let cycle = self.cycle_holders(req, held)?;
        let deadlock = !cycle.is_empty();
        if deadlock {
            self.mark_yield_parties(req, &cycle)?;
        }
        let reason = if deadlock {
            "deadlock"
        } else if blocker.scope == SHARED_SCOPE || req.scope == SHARED_SCOPE {
            "shared"
        } else if blocker.path == req.path {
            "same-path"
        } else {
            "overlap"
        };
        self.record_claim_event(
            req.now,
            &ClaimEvent {
                room: req.room,
                path: req.path,
                event: "refused",
                reason,
                held: 0,
                waited: None,
            },
        );
        self.open_claim_conflict(ClaimKey::new(req.room, req.owner, req.path), req.now);
        Ok(Error::Conflict {
            path: blocker.path.clone(),
            owner: blocker.owner.clone(),
            secs_left: crate::domain::secs_between(blocker.expires, req.now),
            held_secs: held_for(blocker.since, req.now),
            deadlock,
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

    /// Delete the claims whose lease has lapsed, record each as an expiry (#1386) and return them.
    fn sweep_lapsed(&self, now: i64) -> Result<Vec<Lapsed>> {
        let lapsed: Vec<Lapsed> = self.rows(
            "SELECT room,path,owner,scope,since,expires,reason FROM claims WHERE expires<=?1",
            params![now],
            |r| {
                Ok(Lapsed {
                    passive: r.get::<_, String>(6)? == PASSIVE_REASON,
                    room: r.get(0)?,
                    path: r.get(1)?,
                    owner: r.get(2)?,
                    scope: r.get(3)?,
                    held: held_for(r.get(4)?, r.get(5)?),
                    expires: r.get(5)?,
                })
            },
        )?;
        self.exec("DELETE FROM claims WHERE expires<=?1", params![now])?;
        for l in &lapsed {
            self.note_yield_end(&l.room, (&l.owner, &l.path), l.expires)?;
            self.record_claim_event(
                now,
                &ClaimEvent {
                    room: &l.room,
                    path: &l.path,
                    event: if l.passive {
                        "passive_expired"
                    } else {
                        "expired"
                    },
                    reason: "",
                    held: l.held,
                    waited: None,
                },
            );
        }
        Ok(lapsed)
    }

    /// A new hold was granted (#1386): the grant, with the wait it ended, and a takeover when the claim it overlapped
    /// had just lapsed under another member.
    fn record_grant(&self, req: &Attempt, lapsed: &[Lapsed]) {
        let waited =
            self.close_claim_conflicts(ClaimKey::new(req.room, req.owner, req.path), req.now);
        let event = |event, held, waited| ClaimEvent {
            room: req.room,
            path: req.path,
            event,
            reason: "",
            held,
            waited,
        };
        self.record_claim_event(req.now, &event("granted", 0, waited));
        let taken = lapsed
            .iter()
            .filter(|l| {
                l.room == req.room
                    && l.owner != req.owner
                    && overlaps(&l.path, req.path)
                    && scopes_collide(&l.scope, req.scope)
            })
            .map(|l| l.held)
            .max();
        if let Some(held) = taken {
            self.record_claim_event(req.now, &event("takeover", held, None));
        }
    }

    /// Take the claim or say what refused it. The outcome is a value, not an error, so that a refused attempt is
    /// still committed: it is remembered as a wait, which is what lets a later conflict be recognised as closing a
    /// cycle.
    fn attempt(&self, req: &Attempt) -> Result<std::result::Result<(), Error>> {
        let lapsed = self.sweep_lapsed(req.now)?;
        self.exec(
            "DELETE FROM claim_waits WHERE ts<?1",
            params![req.now - WAIT_WINDOW_SECS],
        )?;
        self.exec(
            "DELETE FROM claim_yields WHERE ts<?1",
            params![req.now - YIELD_WINDOW_SECS],
        )?;
        let held = self.live_claims(req.room)?;
        let blocker = held.iter().find(|h| refuses(h, req));
        if let Some(blocker) = blocker {
            return Ok(Err(self.refuse(req, blocker, &held)?));
        }
        let renewal = held
            .iter()
            .any(|h| h.owner == req.owner && h.path == req.path);
        self.end_waits(req)?;
        self.insert_claim(req)?;
        if !renewal {
            self.record_grant(req, &lapsed);
        }
        self.note_overwrites(req)?;
        self.tell_completed_cycles(req)?;
        Ok(Ok(()))
    }

    /// Take a passive claim (#1401) in one transaction: a refusal is returned as a value and records no wait, so the
    /// member that already edited the file never looks like it waits for the holder; a grant and a renewal are counted
    /// as passive events, not as explicit claims.
    pub(super) fn do_passive_claim(
        &self,
        request: &ClaimRequest,
        now: i64,
    ) -> Result<PassiveTaken> {
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
        let passive = |event, held| ClaimEvent {
            room: request.room,
            path: request.path,
            event,
            reason: "",
            held,
            waited: None,
        };
        self.immediate(|| {
            self.sweep_lapsed(now)?;
            let held = self.live_claims(attempt.room)?;
            if let Some(b) = held.iter().find(|h| refuses(h, &attempt)) {
                self.record_claim_event(now, &passive("passive_refused", 0));
                return Ok(PassiveTaken::Refused {
                    owner: b.owner.clone(),
                    path: b.path.clone(),
                    held_secs: held_for(b.since, now),
                    secs_left: crate::domain::secs_between(b.expires, now),
                });
            }
            let renewal = held
                .iter()
                .any(|h| h.owner == attempt.owner && h.path == attempt.path);
            self.end_waits(&attempt)?;
            self.insert_claim(&attempt)?;
            self.record_claim_event(
                now,
                &passive(
                    if renewal {
                        "passive_renewed"
                    } else {
                        "passive_granted"
                    },
                    0,
                ),
            );
            self.note_overwrites(&attempt)?;
            self.tell_completed_cycles(&attempt)?;
            Ok(if renewal {
                PassiveTaken::Renewed
            } else {
                PassiveTaken::Granted
            })
        })
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
    /// The cycles of waits standing at `now`, read only (#271): a wait (a refused claim within the window) whose
    /// blocking holder waits, directly or through others, for something the waiter holds is on a cycle. Waiters and
    /// holders joined by such an edge form one cycle's member set; each set is sorted, and the sets are sorted.
    pub(super) fn standing_wait_cycles(&self, room: &str, now: i64) -> Result<Vec<Vec<String>>> {
        let waits: Vec<Wait> = self.rows(
            "SELECT member,path,scope FROM claim_waits WHERE room=?1 AND ts>=?2",
            params![room, now - WAIT_WINDOW_SECS],
            |r| {
                Ok(Wait {
                    member: r.get(0)?,
                    path: r.get(1)?,
                    scope: r.get(2)?,
                })
            },
        )?;
        let mut held = self.live_claims(room)?;
        held.retain(|h| h.expires > now);
        let mut groups: Vec<Vec<String>> = Vec::new();
        for w in &waits {
            for h in held.iter().filter(|h| {
                h.owner != w.member
                    && overlaps(&h.path, &w.path)
                    && scopes_collide(&h.scope, &w.scope)
                    && reaches(&h.owner, &w.member, &waits, &held)
            }) {
                let (a, b) = (w.member.clone(), h.owner.clone());
                let (hit, rest): (Vec<_>, Vec<_>) = groups
                    .drain(..)
                    .partition(|g| g.contains(&a) || g.contains(&b));
                groups = rest;
                let mut merged: Vec<String> = hit.into_iter().flatten().collect();
                merged.extend([a, b]);
                merged.sort();
                merged.dedup();
                groups.push(merged);
            }
        }
        groups.sort();
        Ok(groups)
    }

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
