//! Who may do what: labels, clearances, roles, room grants, invites and ssh key mappings.

use super::Direct;
use crate::domain::Result;
use crate::gateway::SharedTransport;
use crate::gateway::shared::forward;
use crate::store::Store;

/// Who may do what: labels, clearances, roles, room grants, invites and ssh key mappings.
pub trait Access {
    /// Labels and clearance (epic #9): a room's label, a member's clearance, the agent cap and the label order.
    fn admin_set_room_label(&self, room: &str, label: &str) -> Result<()>;
    fn admin_set_clearance(&self, member: &str, label: &str) -> Result<()>;
    fn admin_set_cap(&self, label: &str) -> Result<()>;
    fn admin_set_librarian(&self, member: &str, on: bool) -> Result<()>;
    fn admin_set_reviewer(&self, member: &str, on: bool) -> Result<()>;
    fn admin_set_facilitator(&self, member: &str, on: bool) -> Result<()>;
    fn admin_set_integrator(&self, member: &str, on: bool) -> Result<()>;
    fn admin_set_admin_agent(&self, member: &str, on: bool) -> Result<()>;
    /// Room creation and invites as a capability (#188): the policy and a member's grant, both the human's.
    fn admin_set_room_creation_granted(&self, on: bool) -> Result<()>;
    fn admin_grant_room_capability(&self, member: &str, on: bool) -> Result<()>;
    /// Invite `member` to `room` with a pointer posted in `via` (#188): needs the capability, is audited.
    fn invite(&self, member: &str, room: &str, via: &str) -> Result<()>;
    /// The room grantor grants `member` a clearance in `room` only, within its own clearance, the room label and the
    /// agent cap (#893): audited, refused with the rule that failed.
    fn grant_room_clearance(&self, room: &str, member: &str, label: &str) -> Result<()>;
    /// The human names the grantor of a room, or takes the role away; and sets or revokes a room grant (#893).
    fn admin_set_room_grantor(&self, room: &str, member: &str, on: bool) -> Result<()>;
    fn admin_set_room_clearance(&self, room: &str, member: &str, label: Option<&str>)
    -> Result<()>;
    /// The human maps an ssh forced-command key label to one member, or takes the mapping away (#107).
    fn admin_set_ssh_key(&self, label: &str, member: &str, on: bool) -> Result<()>;
    fn admin_set_label_chain(&self, names: &[&str]) -> Result<()>;
}

impl<S: Store> Access for Direct<S> {
    fn admin_set_room_label(&self, room: &str, label: &str) -> Result<()> {
        self.gateway
            .admin_set_room_label(&self.principal, room, label)
    }
    fn admin_set_clearance(&self, member: &str, label: &str) -> Result<()> {
        self.gateway
            .admin_set_clearance(&self.principal, member, label)
    }
    fn admin_set_cap(&self, label: &str) -> Result<()> {
        self.gateway.admin_set_cap(&self.principal, label)
    }
    fn admin_set_librarian(&self, member: &str, on: bool) -> Result<()> {
        self.gateway
            .admin_set_librarian(&self.principal, member, on)
    }
    fn admin_set_reviewer(&self, member: &str, on: bool) -> Result<()> {
        self.gateway.admin_set_reviewer(&self.principal, member, on)
    }
    fn admin_set_facilitator(&self, member: &str, on: bool) -> Result<()> {
        self.gateway
            .admin_set_facilitator(&self.principal, member, on)
    }
    fn admin_set_integrator(&self, member: &str, on: bool) -> Result<()> {
        self.gateway
            .admin_set_integrator(&self.principal, member, on)
    }
    fn admin_set_admin_agent(&self, member: &str, on: bool) -> Result<()> {
        self.gateway
            .admin_set_admin_agent(&self.principal, member, on)
    }
    fn admin_set_room_creation_granted(&self, on: bool) -> Result<()> {
        self.gateway
            .admin_set_room_creation_granted(&self.principal, on)
    }
    fn admin_grant_room_capability(&self, member: &str, on: bool) -> Result<()> {
        self.gateway
            .admin_grant_room_capability(&self.principal, member, on)
    }
    fn invite(&self, member: &str, room: &str, via: &str) -> Result<()> {
        self.gateway.invite(&self.principal, member, room, via)
    }
    fn grant_room_clearance(&self, room: &str, member: &str, label: &str) -> Result<()> {
        self.gateway
            .grant_room_clearance(&self.principal, room, member, label)
    }
    fn admin_set_room_grantor(&self, room: &str, member: &str, on: bool) -> Result<()> {
        self.gateway
            .admin_set_room_grantor(&self.principal, room, member, on)
    }
    fn admin_set_room_clearance(
        &self,
        room: &str,
        member: &str,
        label: Option<&str>,
    ) -> Result<()> {
        self.gateway
            .admin_set_room_clearance(&self.principal, room, member, label)
    }
    fn admin_set_ssh_key(&self, label: &str, member: &str, on: bool) -> Result<()> {
        self.gateway
            .admin_set_ssh_key(&self.principal, label, member, on)
    }
    fn admin_set_label_chain(&self, names: &[&str]) -> Result<()> {
        self.gateway.admin_set_label_chain(&self.principal, names)
    }
}

impl Access for SharedTransport {
    forward! {
        admin_set_room_label(room: &str, label: &str) -> Result<()>;
        admin_set_clearance(member: &str, label: &str) -> Result<()>;
        admin_set_cap(label: &str) -> Result<()>;
        admin_set_librarian(member: &str, on: bool) -> Result<()>;
        admin_set_reviewer(member: &str, on: bool) -> Result<()>;
        admin_set_facilitator(member: &str, on: bool) -> Result<()>;
        admin_set_integrator(member: &str, on: bool) -> Result<()>;
        admin_set_admin_agent(member: &str, on: bool) -> Result<()>;
        admin_set_room_creation_granted(on: bool) -> Result<()>;
        admin_grant_room_capability(member: &str, on: bool) -> Result<()>;
        invite(member: &str, room: &str, via: &str) -> Result<()>;
        grant_room_clearance(room: &str, member: &str, label: &str) -> Result<()>;
        admin_set_room_grantor(room: &str, member: &str, on: bool) -> Result<()>;
        admin_set_room_clearance(room: &str, member: &str, label: Option<&str>) -> Result<()>;
        admin_set_ssh_key(label: &str, member: &str, on: bool) -> Result<()>;
    }

    fn admin_set_label_chain(&self, names: &[&str]) -> Result<()> {
        let names: Vec<String> = names.iter().map(|n| (*n).to_owned()).collect();
        self.call(move |t| {
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            t.admin_set_label_chain(&names)
        })
    }
}
