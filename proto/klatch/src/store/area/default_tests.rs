//! The default bodies of the claim and credential areas (#726). SqliteStore overrides every one, so nothing else
//! reaches them; a second store that implements only the required methods gets exactly these.

use super::*;
use crate::domain::{Claim, Error, Result};
use crate::store::{ClaimRequest, NewCredential, Release};
use std::cell::RefCell;

/// A store with only the required methods of the claim area.
#[derive(Default)]
struct Minimal {
    released: RefCell<Vec<(String, String, String)>>,
    live: Vec<Claim>,
}

impl ClaimStore for Minimal {
    fn claim(&self, _request: &ClaimRequest, _now: i64) -> Result<()> {
        Ok(())
    }
    fn release(&self, room: &str, owner: &str, path: &str) -> Result<()> {
        self.released
            .borrow_mut()
            .push((room.into(), owner.into(), path.into()));
        Ok(())
    }
    fn claims(&self, _room: &str, _now: i64) -> Result<Vec<Claim>> {
        Ok(self.live.clone())
    }
}

fn claim((owner, path): (&str, &str)) -> Claim {
    Claim {
        path: path.into(),
        owner: owner.into(),
        reason: String::new(),
        expires: 100,
        scope: String::new(),
    }
}

#[test]
fn a_release_with_a_note_is_a_plain_release_of_the_same_path_when_the_store_keeps_no_notes() {
    let s = Minimal::default();
    s.release_with(&Release::new(("r", "alice"), "p", "moved it"), 5)
        .unwrap();
    assert_eq!(
        *s.released.borrow(),
        vec![("r".to_string(), "alice".to_string(), "p".to_string())]
    );
}

#[test]
fn the_default_claim_area_knows_no_notes_anchors_owners_ages_or_cycles() {
    let s = Minimal::default();
    assert_eq!(s.last_release("r", "p").unwrap(), None);
    assert!(s.release_anchored("a", 5).unwrap().is_empty());
    assert!(s.anchors_in_use(5).unwrap().is_empty());
    assert_eq!(s.release_member_claims("alice", 5).unwrap(), 0);
    assert!(s.claim_owners().unwrap().is_empty());
    assert_eq!(s.retime_member_claims("alice", 9, true).unwrap(), 0);
    assert_eq!(s.retime_member_claims("alice", 9, false).unwrap(), 0);
    assert!(s.claim_ages("r", "alice", 5).unwrap().is_empty());
    assert!(s.deadlock_notices("r", "alice", 5).unwrap().is_empty());
}

#[test]
fn the_default_changes_report_every_other_members_claim_and_nothing_released() {
    let s = Minimal {
        live: vec![
            claim(("alice", "a")),
            claim(("bob", "b")),
            claim(("carol", "c")),
        ],
        ..Minimal::default()
    };
    let changes = s.claim_changes("r", "alice", 5).unwrap();
    let owners: Vec<&str> = changes.added.iter().map(|c| c.owner.as_str()).collect();
    assert_eq!(owners, ["bob", "carol"], "never the asker's own");
    assert!(changes.released.is_empty());
}

/// A store with only the defaults of the credential area.
struct NoCredentials;
impl CredentialStore for NoCredentials {}

#[test]
fn the_default_credential_area_registers_nothing_and_says_so() {
    let s = NoCredentials;
    let c = NewCredential {
        member: "w-1",
        token_hash: &[1; 32],
        clearance: "private",
        parent: "lead",
        ttl: 60,
        max_life: 0,
    };
    assert!(matches!(s.issue_credential(&c, 5), Err(Error::Invalid(_))));
    assert_eq!(s.credential("w-1").unwrap(), None);
    assert!(!s.revoke_credential("w-1", 5).unwrap());
    assert_eq!(s.renew_credential("w-1", 5, 60).unwrap(), None);
    assert!(s.credentials().unwrap().is_empty());
}
