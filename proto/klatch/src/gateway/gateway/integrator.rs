//! The integrator (#865, epic #863): the member that merges for the team, with its own login to the code host on its
//! own machine. It is an ordinary member in everything but one thing: only an integrator (or the human at a
//! terminal) may take the `merge:main` claim (`claims.rs`, rule `integrator`). It holds one role in the same
//! `role:<member>` policy entry as the others, and only the human at a terminal grants it.

use super::roles::RoleChange;
use super::*;

impl<S: Store> Gateway<S> {
    /// Give a member the integrator role, or take it away.
    pub fn admin_set_integrator(&self, who: &Principal, member: &str, on: bool) -> Result<()> {
        self.set_role(
            who,
            RoleChange {
                member,
                role: Role::Integrator,
                on,
            },
        )
    }
}
