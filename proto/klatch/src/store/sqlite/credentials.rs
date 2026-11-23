//! Issuing a worker credential.

use super::*;

/// Whether a stored credential (expiry, revoked flag) still stands at `now`.
fn is_live((expires, revoked): (i64, i64), now: i64) -> bool {
    revoked == 0 && expires > now
}

/// A stored credential as the renewal sees it.
struct Stored {
    expires: i64,
    revoked: i64,
    idle: i64,
    cap: i64,
}

impl Stored {
    /// Whether the credential may be renewed at all at `now`: live, with an idle timeout and a cap (a legacy one has none).
    fn renewable(&self, now: i64) -> bool {
        is_live((self.expires, self.revoked), now) && self.cap > 0 && self.idle > 0
    }

    /// The expiry a renewal at `now` writes, or `None` when there is nothing to give: no gain, or a gain under `step`
    /// that does not land on the cap.
    fn renewed_expiry(&self, now: i64, step: i64) -> Option<i64> {
        if !self.renewable(now) {
            return None;
        }
        let next = (now + self.idle).min(self.cap);
        let worth_writing =
            next > self.expires && (next - self.expires >= step || next == self.cap);
        worth_writing.then_some(next)
    }
}

impl SqliteStore {
    pub(super) fn do_renew_credential(
        &self,
        member: &str,
        now: i64,
        step: i64,
    ) -> Result<Option<i64>> {
        self.immediate(|| {
            let row: Option<Stored> = self
                .conn
                .query_row(
                    "SELECT expires, revoked, idle, cap FROM credentials WHERE member=?1",
                    params![member],
                    |r| {
                        Ok(Stored {
                            expires: r.get(0)?,
                            revoked: r.get(1)?,
                            idle: r.get(2)?,
                            cap: r.get(3)?,
                        })
                    },
                )
                .ok();
            let Some(next) = row.and_then(|c| c.renewed_expiry(now, step)) else {
                return Ok(None);
            };
            self.exec(
                "UPDATE credentials SET expires=?2 WHERE member=?1 AND revoked=0",
                params![member, next],
            )?;
            Ok(Some(next))
        })
    }

    pub(super) fn do_issue_credential(
        &self,
        c: &crate::store::NewCredential,
        now: i64,
    ) -> Result<()> {
        self.immediate(|| {
            let live: Option<(i64, i64)> = self
                .conn
                .query_row(
                    "SELECT expires, revoked FROM credentials WHERE member=?1",
                    params![c.member],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok();
            if live.is_some_and(|held| is_live(held, now)) {
                return Err(Error::Invalid(format!(
                    "{} already holds a live credential; revoke it first or wait for it to expire",
                    c.member
                )));
            }
            // with a cap the first expiry is the idle timeout or the cap, whichever is nearer; without one, the ttl
            let life = if c.max_life > 0 { c.ttl.min(c.max_life) } else { c.ttl };
            let cap = if c.max_life > 0 { now + c.max_life } else { 0 };
            let idle = if c.max_life > 0 { c.ttl } else { 0 };
            self.exec(
                "INSERT INTO credentials(member,token_hash,clearance,parent,issued,expires,revoked,idle,cap) VALUES(?1,?2,?3,?4,?5,?6,0,?7,?8)
                 ON CONFLICT(member) DO UPDATE SET token_hash=?2, clearance=?3, parent=?4, issued=?5, expires=?6, revoked=0, idle=?7, cap=?8",
                params![c.member, c.token_hash, c.clearance, c.parent, now, now + life, idle, cap],
            )
            .map(|_| ())
        })
    }
}
