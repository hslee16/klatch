//! Administrative acts on rooms, runs and members, and the refusal and audit logs.

use super::Direct;
use crate::domain::Result;
use crate::gateway::SharedTransport;
use crate::gateway::shared::forward;
use crate::store::Store;

/// Administrative acts on rooms, runs and members, and the refusal and audit logs.
pub trait Admin {
    /// Administrative acts (`klatch admin`, R11); who may call them is the caller's decision, not the gateway's.
    fn admin_close_room(&self, room: &str, closed: bool) -> Result<()>;
    fn admin_pause(&self, paused: bool) -> Result<()>;
    fn admin_remove_member(&self, room: &str, member: &str) -> Result<()>;
    fn admin_allow_member(&self, room: &str, member: &str) -> Result<()>;
    fn admin_audit(&self, limit: usize) -> Result<Vec<crate::domain::AuditRecord>>;
    /// Open a run: its rooms (`run.<id>.<name>`) may be used from now on (#746).
    fn admin_run_create(&self, run: &str) -> Result<()>;
    /// End a run and delete its rooms, with no archive (a caller over the wire never names a file); returns how many
    /// rooms and messages went (#746).
    fn admin_run_end(&self, run: &str) -> Result<crate::store::DeletedRooms>;
    /// The latest refused calls, newest first.
    fn admin_refusals(&self, limit: usize) -> Result<Vec<crate::domain::RefusalRecord>>;
    /// The caller's own latest refusals, newest first (#678); never another member's.
    fn my_refusals(&self, limit: usize) -> Result<Vec<crate::domain::RefusalRecord>>;
}

impl<S: Store> Admin for Direct<S> {
    fn admin_close_room(&self, room: &str, closed: bool) -> Result<()> {
        self.gateway.admin_close_room(&self.principal, room, closed)
    }
    fn admin_pause(&self, paused: bool) -> Result<()> {
        self.gateway.admin_pause(&self.principal, paused)
    }
    fn admin_remove_member(&self, room: &str, member: &str) -> Result<()> {
        self.gateway
            .admin_remove_member(&self.principal, room, member)
    }
    fn admin_allow_member(&self, room: &str, member: &str) -> Result<()> {
        self.gateway
            .admin_allow_member(&self.principal, room, member)
    }
    fn admin_audit(&self, limit: usize) -> Result<Vec<crate::domain::AuditRecord>> {
        self.gateway.admin_audit(&self.principal, limit)
    }
    fn admin_run_create(&self, run: &str) -> Result<()> {
        self.gateway.admin_run_create(&self.principal, run)
    }
    fn admin_run_end(&self, run: &str) -> Result<crate::store::DeletedRooms> {
        self.gateway.admin_run_end(&self.principal, run, None)
    }
    fn admin_refusals(&self, limit: usize) -> Result<Vec<crate::domain::RefusalRecord>> {
        self.gateway.admin_refusals(&self.principal, limit)
    }
    fn my_refusals(&self, limit: usize) -> Result<Vec<crate::domain::RefusalRecord>> {
        self.gateway.my_refusals(&self.principal, limit)
    }
}

impl Admin for SharedTransport {
    forward! {
        admin_close_room(room: &str, closed: bool) -> Result<()>;
        admin_pause(paused: bool) -> Result<()>;
        admin_remove_member(room: &str, member: &str) -> Result<()>;
        admin_allow_member(room: &str, member: &str) -> Result<()>;
        admin_audit(limit: usize) -> Result<Vec<crate::domain::AuditRecord>>;
        admin_run_create(run: &str) -> Result<()>;
        admin_run_end(run: &str) -> Result<crate::store::DeletedRooms>;
        admin_refusals(limit: usize) -> Result<Vec<crate::domain::RefusalRecord>>;
        my_refusals(limit: usize) -> Result<Vec<crate::domain::RefusalRecord>>;
    }
}
