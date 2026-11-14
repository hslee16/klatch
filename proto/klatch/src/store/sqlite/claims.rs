//! Taking a claim: the conflict check, the record of who is waiting for whom (so a conflict that closes a cycle is
//! recognised as a deadlock), and the short lease an `ask:<N>` claim carries.

use super::{SqliteStore, WAIT_WINDOW_SECS};
use crate::domain::{Error, Result, overlaps, scopes_collide};
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

/// The ask number in an `ask:<N>` claim path.
fn ask_number(path: &str) -> Option<i64> {
    path.strip_prefix("ask:").and_then(|n| n.parse().ok())
}

impl SqliteStore {
    fn live_claims(&self, room: &str) -> Result<Vec<Held>> {
        self.rows(
            "SELECT path,owner,expires,scope FROM claims WHERE room=?1",
            params![room],
            |r| {
                Ok(Held {
                    path: r.get(0)?,
                    owner: r.get(1)?,
                    expires: r.get(2)?,
                    scope: r.get(3)?,
                })
            },
        )
    }

    /// Whether a refusal of `requester` by `holder` closes a cycle of waits: `holder` is waiting (a refused claim
    /// attempt within the window) for something a member holds who is waiting for something ... that `requester`
    /// holds.
    fn closes_cycle(
        &self,
        room: &str,
        (requester, holder): (&str, &str),
        held: &[Held],
    ) -> Result<bool> {
        let waits = self.rows(
            "SELECT member,path,scope FROM claim_waits WHERE room=?1",
            params![room],
            |r| {
                Ok(Wait {
                    member: r.get(0)?,
                    path: r.get(1)?,
                    scope: r.get(2)?,
                })
            },
        )?;
        Ok(reaches(holder, requester, &waits, held))
    }

    /// Remember that the requester is waiting for the path, and say what refused it.
    fn refuse(&self, req: &Attempt, blocker: &Held, held: &[Held]) -> Result<Error> {
        self.exec(
            "INSERT INTO claim_waits(room,member,path,ts,scope) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(room,member,path) DO UPDATE SET ts=?4, scope=?5",
            params![req.room, req.owner, req.path, req.now, req.scope],
        )?;
        let parties = (req.owner, blocker.owner.as_str());
        Ok(Error::Conflict {
            path: blocker.path.clone(),
            owner: blocker.owner.clone(),
            secs_left: crate::domain::secs_between(blocker.expires, req.now),
            deadlock: self.closes_cycle(req.room, parties, held)?,
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

    fn insert_claim(&self, req: &Attempt) -> Result<()> {
        self.exec(
            "DELETE FROM claims WHERE room=?1 AND owner=?2 AND path=?3",
            params![req.room, req.owner, req.path],
        )?;
        self.exec(
            "INSERT INTO claims(room,path,owner,member,reason,expires,scope,anchor) VALUES(?1,?2,?3,?3,?4,?5,?6,?7)",
            params![req.room, req.path, req.owner, req.reason, req.now + req.ttl, req.scope, req.anchor],
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
        let blocker = held.iter().find(|h| {
            h.owner != req.owner
                && overlaps(&h.path, req.path)
                && scopes_collide(&h.scope, req.scope)
        });
        if let Some(blocker) = blocker {
            return Ok(Err(self.refuse(req, blocker, &held)?));
        }
        self.end_waits(req)?;
        self.insert_claim(req)?;
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
