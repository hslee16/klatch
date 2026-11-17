//! Issuing a worker credential.

use super::*;

/// Whether a stored credential (expiry, revoked flag) still stands at `now`.
fn is_live((expires, revoked): (i64, i64), now: i64) -> bool {
    revoked == 0 && expires > now
}

impl SqliteStore {
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
            self.exec(
                "INSERT INTO credentials(member,token_hash,clearance,parent,issued,expires,revoked) VALUES(?1,?2,?3,?4,?5,?6,0)
                 ON CONFLICT(member) DO UPDATE SET token_hash=?2, clearance=?3, parent=?4, issued=?5, expires=?6, revoked=0",
                params![c.member, c.token_hash, c.clearance, c.parent, now, now + c.ttl],
            )
            .map(|_| ())
        })
    }
}
