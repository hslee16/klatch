//! #1090: a member reads its own role (and no one else's), so a dry run can prove the integrator role before the
//! first `merge:main` claim instead of inferring it from a refusal.

use super::*;

#[test]
fn a_member_reads_its_own_role_as_a_word_and_none_without_one() {
    let (g, _) = timed();
    g.join(&alice(), "lobby").unwrap();
    assert_eq!(g.my_role(&alice()).unwrap(), None);
    g.admin_set_integrator(&admin(), "alice", true).unwrap();
    assert_eq!(g.my_role(&alice()).unwrap().as_deref(), Some("integrator"));
    g.admin_set_reviewer(&admin(), "alice", true).unwrap();
    assert_eq!(g.my_role(&alice()).unwrap().as_deref(), Some("reviewer"));
    g.admin_set_reviewer(&admin(), "alice", false).unwrap();
    assert_eq!(g.my_role(&alice()).unwrap(), None);
}

#[test]
fn another_members_role_does_not_leak_into_the_read() {
    let (g, _) = timed();
    g.join(&alice(), "lobby").unwrap();
    g.join(&bob(), "lobby").unwrap();
    g.admin_set_integrator(&admin(), "alice", true).unwrap();
    assert_eq!(g.my_role(&bob()).unwrap(), None);
}
