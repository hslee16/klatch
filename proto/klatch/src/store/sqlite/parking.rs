//! A member stepping away (park) and coming back (unpark): what each does to its leases.

use super::*;
use crate::domain::{MERGE_CLAIM, MERGE_PARK_MAX_SECS};

impl SqliteStore {
    pub(super) fn do_park(&self, member: &str, parked: &Parked, now: i64) -> Result<()> {
        self.immediate(|| {
            // What an earlier park still holds: its return time, if that is still ahead (#647).
            let held_until: i64 = self
                .optional(
                    "SELECT back_by FROM member_parked WHERE member=?1",
                    params![member],
                    |r| r.get(0),
                )?
                .unwrap_or(0);
            self.exec(
                "INSERT INTO member_parked(member,why,since,back_by) VALUES(?1,?2,?3,?4)
                 ON CONFLICT(member) DO UPDATE SET why=?2, since=?3, back_by=?4",
                params![member, parked.why, now, parked.back_by],
            )?;
            // Only what is live now is held: a claim that already ran out stays gone. A claim short of the return
            // time is stretched to it. One beyond the return time (or its own ttl from now, item 52, if later) but
            // within what the earlier park held is that park's stretch, and is cut back to it; a claim beyond the
            // earlier return time was never stretched by it and stays.
            let target = "MAX(?3, ?2+ttl)";
            self.exec(
                &format!(
                    "UPDATE claims SET expires=CASE WHEN expires<?3 THEN ?3 ELSE {target} END
                     WHERE owner=?1 AND expires>?2 AND path<>?5
                       AND (expires<?3 OR (expires>{target} AND expires<=?4))"
                ),
                params![member, now, parked.back_by, held_until, MERGE_CLAIM],
            )?;
            // The merge claim is stretched only to the park bound, not to the return time, and never shortened here
            // (#865): a holder that waits for an event and goes silent lapses at the bound.
            self.exec(
                "UPDATE claims SET expires=MIN(?3, ?2+?4) WHERE owner=?1 AND path=?5 AND expires>?2
                 AND expires<MIN(?3, ?2+?4)",
                params![
                    member,
                    now,
                    parked.back_by,
                    MERGE_PARK_MAX_SECS,
                    MERGE_CLAIM
                ],
            )?;
            self.exec(
                "UPDATE ask_leases SET expires=?3 WHERE owner=?1 AND expires>?2 AND expires<?3",
                params![member, now, parked.back_by],
            )?;
            // An ask lease does not outlive its claim row, which a re-park may have cut.
            self.exec(
                "UPDATE ask_leases SET expires=(SELECT c.expires FROM claims c WHERE c.room=ask_leases.room
                     AND c.path='ask:'||ask_leases.seq AND c.owner=ask_leases.owner)
                 WHERE owner=?1 AND expires>?2 AND EXISTS (SELECT 1 FROM claims c WHERE c.room=ask_leases.room
                     AND c.path='ask:'||ask_leases.seq AND c.owner=ask_leases.owner
                     AND c.expires<ask_leases.expires AND c.expires>?2)",
                params![member, now],
            )?;
            Ok(())
        })
    }

    pub(super) fn do_unpark(&self, member: &str, now: i64) -> Result<()> {
        self.immediate(|| {
            if self.exec("DELETE FROM member_parked WHERE member=?1", params![member])? == 0 {
                return Ok(());
            }
            // A member that is active again renews for itself: what the park stretched is cut back to a normal
            // lease, short for an ask. Never below the claim's own ttl from now (item 52, #649): the post's renewal
            // already ran, so a cut under the ttl would leave a long claim short until the next action.
            let (plain, ask) = (now + CLAIM_DEFAULT_SECS, now + ASK_LEASE_DEFAULT_SECS);
            self.exec(
                "UPDATE claims SET expires=?4+MAX(ttl, CASE WHEN path LIKE 'ask:%' THEN ?3 ELSE ?2 END)
                 WHERE owner=?1 AND expires>?4+MAX(ttl, CASE WHEN path LIKE 'ask:%' THEN ?3 ELSE ?2 END)",
                params![member, CLAIM_DEFAULT_SECS, ASK_LEASE_DEFAULT_SECS, now],
            )?;
            self.exec(
                "UPDATE ask_leases SET expires=?3 WHERE owner=?1 AND expires>?3",
                params![member, plain, ask],
            )?;
            Ok(())
        })
    }
}
