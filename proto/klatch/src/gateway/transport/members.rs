//! Who is in a room and what a caller says about itself: roster, scope, role, status, credential, housekeeping.

use super::Direct;
use crate::domain::MemberInfo;
use crate::domain::Result;
use crate::gateway::SharedTransport;
use crate::gateway::shared::forward;
use crate::store::Store;

/// Who is in a room and what a caller says about itself: roster, scope, role, status, credential, housekeeping.
pub trait Members {
    fn who(&self, room: &str) -> Result<Vec<MemberInfo>>;
    /// Declare the scope the caller works in.
    fn set_scope(&self, scope: &str) -> Result<()>;
    /// The scope the caller declared or was given.
    fn scope(&self) -> Result<Option<String>>;
    /// The role the caller holds, as its word, or `None` (#1090).
    fn my_role(&self) -> Result<Option<String>>;
    /// A worker's session ended: revoke the caller's own credential (nothing for a caller that holds none).
    fn end_credential(&self) -> Result<bool>;
    /// Housekeeping: remove members not seen for `older_than` seconds; returns how many.
    fn prune_members(&self, older_than: i64) -> Result<usize>;
    /// Set the caller's own status in a room.
    fn set_status(&self, room: &str, status: &str) -> Result<()>;
    /// The rooms the caller has joined, read without consuming any message.
    fn rooms(&self) -> Result<Vec<String>>;
    /// Whether the process behind a room member's session has exited, by the liveness probe (#790). False for
    /// anything the probe cannot say, so unknown counts as alive.
    fn process_exited(&self, room: &str, member: &str) -> Result<bool>;
}

impl<S: Store> Members for Direct<S> {
    fn who(&self, room: &str) -> Result<Vec<MemberInfo>> {
        self.gateway.who(&self.principal, room)
    }
    fn set_scope(&self, scope: &str) -> Result<()> {
        self.gateway.set_scope(&self.principal, scope)
    }
    fn scope(&self) -> Result<Option<String>> {
        self.gateway.scope_of(&self.principal)
    }
    fn my_role(&self) -> Result<Option<String>> {
        self.gateway.my_role(&self.principal)
    }
    fn end_credential(&self) -> Result<bool> {
        self.gateway.end_credential(&self.principal)
    }
    fn prune_members(&self, older_than: i64) -> Result<usize> {
        self.gateway.prune_members(&self.principal, older_than)
    }
    fn set_status(&self, room: &str, status: &str) -> Result<()> {
        self.gateway.set_status(&self.principal, room, status)
    }
    fn rooms(&self) -> Result<Vec<String>> {
        self.gateway.rooms(&self.principal)
    }
    fn process_exited(&self, room: &str, member: &str) -> Result<bool> {
        self.gateway.process_exited(&self.principal, room, member)
    }
}

impl Members for SharedTransport {
    forward! {
        who(room: &str) -> Result<Vec<crate::domain::MemberInfo>>;
        set_scope(scope: &str) -> Result<()>;
        scope() -> Result<Option<String>>;
        my_role() -> Result<Option<String>>;
        end_credential() -> Result<bool>;
        prune_members(older_than: i64) -> Result<usize>;
        set_status(room: &str, status: &str) -> Result<()>;
        rooms() -> Result<Vec<String>>;
        process_exited(room: &str, member: &str) -> Result<bool>;
    }
}
