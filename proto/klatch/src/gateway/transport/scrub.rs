//! The leak guards: canaries and the scrubber's private terms.

use super::Direct;
use crate::domain::Result;
use crate::gateway::SharedTransport;
use crate::gateway::shared::forward;
use crate::store::Store;

/// The leak guards: canaries and the scrubber's private terms.
pub trait Scrub {
    /// Plant a canary, a fake secret, in a room (#46); text carrying it into any other room trips a wire.
    fn admin_plant_canary(&self, room: &str) -> Result<()>;
    /// End the canary of a room (#347).
    fn admin_retire_canary(&self, room: &str) -> Result<()>;
    /// The scrubber (#44): list a private term under a rule name, stop it, and show what is listed.
    fn admin_add_scrub_term(&self, name: &str, term: &str) -> Result<()>;
    fn admin_remove_scrub_term(&self, name: &str) -> Result<()>;
    fn admin_scrub_terms(&self) -> Result<Vec<(String, String)>>;
}

impl<S: Store> Scrub for Direct<S> {
    fn admin_plant_canary(&self, room: &str) -> Result<()> {
        self.gateway.admin_plant_canary(&self.principal, room)
    }
    fn admin_retire_canary(&self, room: &str) -> Result<()> {
        self.gateway.admin_retire_canary(&self.principal, room)
    }
    fn admin_add_scrub_term(&self, name: &str, term: &str) -> Result<()> {
        self.gateway
            .admin_add_scrub_term(&self.principal, name, term)
    }
    fn admin_remove_scrub_term(&self, name: &str) -> Result<()> {
        self.gateway.admin_remove_scrub_term(&self.principal, name)
    }
    fn admin_scrub_terms(&self) -> Result<Vec<(String, String)>> {
        self.gateway.admin_scrub_terms(&self.principal)
    }
}

impl Scrub for SharedTransport {
    forward! {
        admin_plant_canary(room: &str) -> Result<()>;
        admin_retire_canary(room: &str) -> Result<()>;
        admin_add_scrub_term(name: &str, term: &str) -> Result<()>;
        admin_remove_scrub_term(name: &str) -> Result<()>;
        admin_scrub_terms() -> Result<Vec<(String, String)>>;
    }
}
