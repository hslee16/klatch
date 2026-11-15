//! The read-only librarian (R30, DESIGN D4): the interface a human uses to ask what happened. The role is a policy
//! entry the human grants with `klatch admin librarian`. A librarian may look (read without moving a cursor, who,
//! claims, board, subscribe, and the audit and refusal records) and nothing else, so it can neither post, claim
//! nor change any policy. It passes through the same stops and label checks as any member, and its clearance is
//! whatever the human granted (the lowest by default).

use super::*;

impl<S: Store> Gateway<S> {
    /// Give a member the librarian role, or take it away.
    pub fn admin_set_librarian(&self, who: &Principal, member: &str, on: bool) -> Result<()> {
        if !valid_name(member) {
            return Err(Error::Invalid(format!("bad member name: {member}")));
        }
        self.administer(who, ("librarian", &format!("{member}={on}")), |now| {
            self.store
                .policy_set(&format!("role:{member}"), on.then_some("librarian"), now)?;
            Ok(String::new())
        })
    }

    /// Whether the member holds the librarian role.
    pub fn is_librarian(&self, member: &str) -> Result<bool> {
        Ok(self.store.policy_get(&format!("role:{member}"))?.as_deref() == Some("librarian"))
    }

    /// The one place every operation reaches the guard: a librarian is refused anything that is not a query
    /// before the guard is asked, so no guard can be configured to let a librarian write.
    pub(super) fn check_guard(&self, who: &Principal, op: &Operation) -> Result<()> {
        if !op.is_query() && self.is_librarian(&who.member)? {
            return Err(Error::Refused {
                rule: "read-only".into(),
                reason: format!("{} is a librarian, which may only look", who.member),
            });
        }
        self.guard.check(who, op)
    }
}
