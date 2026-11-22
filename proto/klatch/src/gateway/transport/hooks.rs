//! What the harness hooks report to the gateway: hook runs, permission mode, session, stop and signs of life.

use super::Direct;
use crate::domain::Result;
use crate::gateway::SharedTransport;
use crate::gateway::shared::forward;
use crate::store::Store;

/// What the harness hooks report to the gateway: hook runs, permission mode, session, stop and signs of life.
pub trait Hooks {
    /// Record a hook run (event and bytes printed) for the caller.
    fn record_hook_run(&self, event: &str, bytes: usize) -> Result<()>;
    /// The latest run of each hook event recorded for `member`.
    fn hook_runs(&self, member: &str) -> Result<Vec<crate::domain::HookRun>>;
    /// Remember the permission mode the caller's session reported in a hook input (#553).
    fn note_member_mode(&self, mode: &str) -> Result<()>;
    /// Remember that the caller is the member of harness session `session_id`.
    fn note_session(&self, session_id: &str) -> Result<()>;
    /// The Stop hook: end any park, then park the caller if background tasks or crons still run.
    fn record_stop(&self, background_tasks: usize, crons: usize) -> Result<()>;
    /// Any other hook ran: a sign of life, which ends a park.
    fn sign_of_life(&self) -> Result<()>;
}

impl<S: Store> Hooks for Direct<S> {
    fn record_hook_run(&self, event: &str, bytes: usize) -> Result<()> {
        self.gateway.record_hook_run(&self.principal, event, bytes)
    }
    fn hook_runs(&self, member: &str) -> Result<Vec<crate::domain::HookRun>> {
        self.gateway.hook_runs(&self.principal, member)
    }
    fn note_member_mode(&self, mode: &str) -> Result<()> {
        self.gateway.note_member_mode(&self.principal, mode)
    }
    fn note_session(&self, session_id: &str) -> Result<()> {
        self.gateway.note_session(&self.principal, session_id)
    }
    fn record_stop(&self, background_tasks: usize, crons: usize) -> Result<()> {
        self.gateway
            .record_stop(&self.principal, background_tasks, crons)
    }
    fn sign_of_life(&self) -> Result<()> {
        self.gateway.sign_of_life(&self.principal)
    }
}

impl Hooks for SharedTransport {
    forward! {
        record_hook_run(event: &str, bytes: usize) -> Result<()>;
        hook_runs(member: &str) -> Result<Vec<crate::domain::HookRun>>;
        note_member_mode(mode: &str) -> Result<()>;
        note_session(session_id: &str) -> Result<()>;
        record_stop(background_tasks: usize, crons: usize) -> Result<()>;
        sign_of_life() -> Result<()>;
    }
}
