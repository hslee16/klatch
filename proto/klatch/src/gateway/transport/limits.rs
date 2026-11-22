//! Rate limits, lease caps and grace, per-room stale and ask lease settings, and capability requirements.

use super::Direct;
use crate::domain::Result;
use crate::gateway::SharedTransport;
use crate::gateway::shared::forward;
use crate::store::Store;

/// Rate limits, lease caps and grace, per-room stale and ask lease settings, and capability requirements.
pub trait Limits {
    /// Limit every member to `posts` posts in `secs` seconds over all rooms, or with `None` lift it (#187).
    fn admin_set_rate_limit(&self, limit: Option<(usize, i64)>) -> Result<()>;
    /// One member's own limit, over the global one; `None` makes it inherit again (#972).
    fn admin_set_member_rate_limit(&self, member: &str, limit: Option<(usize, i64)>) -> Result<()>;
    /// The global limit (member `None`) and every member limit in force, sorted by name (#1023).
    fn admin_rate_limits(&self) -> Result<Vec<crate::gateway::RateLimitEntry>>;
    /// The lease cap and the grace period, in seconds, or with `None` back to the default (#537).
    fn admin_set_lease_cap(&self, secs: Option<i64>) -> Result<()>;
    fn admin_set_lease_grace(&self, secs: Option<i64>) -> Result<()>;
    /// A room's stale-ask threshold and its default `ask:` claim lease, in seconds, or with `None` back to the
    /// built-in value (#242).
    fn admin_set_room_stale(&self, room: &str, secs: Option<i64>) -> Result<()>;
    fn admin_set_room_ask_lease(&self, room: &str, secs: Option<i64>) -> Result<()>;
    /// Require harness capabilities of a room as `refuse:CAPS` or `flag:CAPS`, or with `None` lift it (#61).
    fn admin_set_room_require(&self, room: &str, spec: Option<&str>) -> Result<()>;
}

impl<S: Store> Limits for Direct<S> {
    fn admin_set_rate_limit(&self, limit: Option<(usize, i64)>) -> Result<()> {
        self.gateway.admin_set_rate_limit(&self.principal, limit)
    }
    fn admin_set_member_rate_limit(&self, member: &str, limit: Option<(usize, i64)>) -> Result<()> {
        self.gateway
            .admin_set_member_rate_limit(&self.principal, member, limit)
    }
    fn admin_rate_limits(&self) -> Result<Vec<crate::gateway::RateLimitEntry>> {
        self.gateway.admin_rate_limits(&self.principal)
    }
    fn admin_set_lease_cap(&self, secs: Option<i64>) -> Result<()> {
        self.gateway.admin_set_lease_cap(&self.principal, secs)
    }
    fn admin_set_lease_grace(&self, secs: Option<i64>) -> Result<()> {
        self.gateway.admin_set_lease_grace(&self.principal, secs)
    }
    fn admin_set_room_stale(&self, room: &str, secs: Option<i64>) -> Result<()> {
        self.gateway
            .admin_set_room_stale(&self.principal, room, secs)
    }
    fn admin_set_room_ask_lease(&self, room: &str, secs: Option<i64>) -> Result<()> {
        self.gateway
            .admin_set_room_ask_lease(&self.principal, room, secs)
    }
    fn admin_set_room_require(&self, room: &str, spec: Option<&str>) -> Result<()> {
        self.gateway
            .admin_set_room_require(&self.principal, room, spec)
    }
}

impl Limits for SharedTransport {
    forward! {
        admin_set_rate_limit(limit: Option<(usize, i64)>) -> Result<()>;
        admin_set_member_rate_limit(member: &str, limit: Option<(usize, i64)>) -> Result<()>;
        admin_rate_limits() -> Result<Vec<crate::gateway::RateLimitEntry>>;
        admin_set_lease_cap(secs: Option<i64>) -> Result<()>;
        admin_set_lease_grace(secs: Option<i64>) -> Result<()>;
        admin_set_room_stale(room: &str, secs: Option<i64>) -> Result<()>;
        admin_set_room_ask_lease(room: &str, secs: Option<i64>) -> Result<()>;
        admin_set_room_require(room: &str, spec: Option<&str>) -> Result<()>;
    }
}
