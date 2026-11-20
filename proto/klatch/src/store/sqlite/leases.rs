//! Moving the expiry of a member's claims, for the lease rules that follow the liveness probe (#304, DESIGN section
//! 29 item 4). A claim row stays in the table after it expires until some claim attempt deletes it, so a holder that
//! the probe finds alive can have a claim that ran out during a long tool call put back before anyone takes the path.

use super::SqliteStore;
use crate::domain::Result;
use rusqlite::params;

impl SqliteStore {
    /// Every member that holds a claim row, expired or not, ordered by name.
    pub(super) fn claim_owners_all(&self) -> Result<Vec<String>> {
        self.rows(
            "SELECT DISTINCT owner FROM claims ORDER BY owner",
            params![],
            |r| r.get(0),
        )
    }

    /// Set the expiry of `member`'s claims to `until` where it is earlier (`extend`) or later (otherwise). An
    /// `ask:<N>` claim keeps its own short lease and is left alone, and so does `merge:main` when it is extended: a
    /// holder the probe sees waiting on a tool is not merging, and the claim must not outlive its lease (#865). Returns how many rows changed.
    pub(super) fn retime_claims(&self, member: &str, until: i64, extend: bool) -> Result<usize> {
        let sql = if extend {
            "UPDATE claims SET expires=?2 WHERE owner=?1 AND expires<?2 AND path NOT LIKE 'ask:%'
             AND path<>'merge:main'"
        } else {
            "UPDATE claims SET expires=?2 WHERE owner=?1 AND expires>?2 AND path NOT LIKE 'ask:%'"
        };
        self.exec(sql, params![member, until])
    }
}

#[cfg(test)]
mod tests {
    use crate::store::areas::*;
    use crate::store::{ClaimRequest, SqliteStore};

    fn store(tag: &str) -> crate::store::test_dir::In<SqliteStore> {
        crate::store::test_dir::In::new(&format!("leases-{tag}"), |dir| {
            SqliteStore::open(&dir.join("rooms.db"), &crate::store::insecure_test_key()).unwrap()
        })
    }

    fn expiry(s: &SqliteStore, room: &str, now: i64, path: &str) -> Option<i64> {
        s.claims(room, now)
            .unwrap()
            .into_iter()
            .find(|c| c.path == path)
            .map(|c| c.expires)
    }

    #[test]
    fn owners_include_a_holder_whose_claim_has_expired_but_not_been_deleted() {
        let s = store("owners");
        s.claim(&ClaimRequest::new(("r", "alice"), "a.rs", 10, ""), 0)
            .unwrap();
        s.claim(&ClaimRequest::new(("r", "bob"), "b.rs", 1000, ""), 0)
            .unwrap();
        assert_eq!(
            s.claim_owners().unwrap(),
            vec!["alice".to_string(), "bob".to_string()]
        );
        assert_eq!(
            s.claims("r", 500).unwrap().len(),
            1,
            "alice's has expired but still has a row"
        );
    }

    #[test]
    fn extending_puts_back_a_claim_that_ran_out_before_anyone_took_it() {
        let s = store("revive");
        s.claim(&ClaimRequest::new(("r", "alice"), "a.rs", 10, ""), 0)
            .unwrap();
        assert_eq!(expiry(&s, "r", 500, "a.rs"), None);
        assert_eq!(s.retime_member_claims("alice", 900, true).unwrap(), 1);
        assert_eq!(expiry(&s, "r", 500, "a.rs"), Some(900));
    }

    #[test]
    fn extending_never_shortens_and_shortening_never_lengthens() {
        let s = store("direction");
        s.claim(&ClaimRequest::new(("r", "alice"), "a.rs", 1000, ""), 0)
            .unwrap();
        s.retime_member_claims("alice", 400, true).unwrap();
        assert_eq!(expiry(&s, "r", 1, "a.rs"), Some(1000));
        s.retime_member_claims("alice", 300, false).unwrap();
        assert_eq!(expiry(&s, "r", 1, "a.rs"), Some(300));
        s.retime_member_claims("alice", 800, false).unwrap();
        assert_eq!(expiry(&s, "r", 1, "a.rs"), Some(300));
    }

    #[test]
    fn only_that_members_claims_move_and_ask_claims_are_left_alone() {
        let s = store("scope");
        s.claim(&ClaimRequest::new(("r", "alice"), "a.rs", 10, ""), 0)
            .unwrap();
        s.claim(&ClaimRequest::new(("r", "alice"), "ask:7", 10, ""), 0)
            .unwrap();
        s.claim(&ClaimRequest::new(("r", "bob"), "b.rs", 10, ""), 0)
            .unwrap();
        assert_eq!(s.retime_member_claims("alice", 900, true).unwrap(), 1);
        assert_eq!(expiry(&s, "r", 5, "ask:7"), Some(10));
        assert_eq!(expiry(&s, "r", 5, "b.rs"), Some(10));
    }

    #[test]
    fn the_probe_never_stretches_the_merge_claim_but_can_still_shorten_it() {
        // a holder seen waiting on a tool is not merging (#865, #533)
        let s = store("merge");
        s.claim(&ClaimRequest::new(("r", "ivy"), "merge:main", 600, ""), 0)
            .unwrap();
        s.claim(&ClaimRequest::new(("r", "ivy"), "a.rs", 600, ""), 0)
            .unwrap();
        assert_eq!(s.retime_member_claims("ivy", 5_000, true).unwrap(), 1);
        assert_eq!(expiry(&s, "r", 1, "merge:main"), Some(600));
        assert_eq!(expiry(&s, "r", 1, "a.rs"), Some(5_000));
        s.retime_member_claims("ivy", 300, false).unwrap();
        assert_eq!(expiry(&s, "r", 1, "merge:main"), Some(300));
    }
}
