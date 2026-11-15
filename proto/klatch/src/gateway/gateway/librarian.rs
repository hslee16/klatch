//! The read-only librarian (R30, DESIGN D4): the interface a human uses to ask what happened. The role is a policy
//! entry the human grants with `klatch admin librarian`. A librarian may look (read without moving a cursor, who,
//! claims, board, subscribe, and the audit and refusal records) and nothing else, so it can neither post, claim
//! nor change any policy. It passes through the same stops and label checks as any member, and its clearance is
//! whatever the human granted (the lowest by default). The role mechanism it shares with the admin agent is in
//! `roles.rs`.

use super::*;

impl<S: Store> Gateway<S> {
    /// Give a member the librarian role, or take it away.
    pub fn admin_set_librarian(&self, who: &Principal, member: &str, on: bool) -> Result<()> {
        self.set_role(who, member, Role::Librarian, on)
    }

    /// Whether the member holds the librarian role.
    pub fn is_librarian(&self, member: &str) -> Result<bool> {
        Ok(self.role_of(member)? == Some(Role::Librarian))
    }
}
