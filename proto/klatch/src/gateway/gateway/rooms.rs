//! Room creation and invites as a capability (#188, R47). Making a room (by joining or posting to one that does not
//! exist) and inviting a member are acts the guard decides and the audit records, with the member's name.
//!
//! The policy is `open` until the human says `granted` (`klatch admin room-creation granted`): with it, only a member
//! holding the capability (`klatch admin grant-rooms <member>`) may create a room or invite. Open is the default so
//! nothing changes for anyone who has not chosen otherwise (flipping the default is a human decision). Only the human
//! at a terminal sets either: both acts are outside the admin agent's list, and a member is refused them.

use super::*;
use crate::store::NewAudit;

const POLICY: &str = "room-creation";
const GRANT: &str = "room-cap:";

impl<S: Store> Gateway<S> {
    /// Only the human: require the capability to create rooms and invite (`on`), or leave it open to everyone.
    pub fn admin_set_room_creation_granted(&self, who: &Principal, on: bool) -> Result<()> {
        let target = if on { "granted" } else { "open" };
        self.administer(who, ("room-creation", target), |now| {
            self.store
                .policy_set(POLICY, on.then_some("granted"), now)?;
            Ok(String::new())
        })
    }

    /// Only the human: give a member the capability to create rooms and invite, or take it away.
    pub fn admin_grant_room_capability(
        &self,
        who: &Principal,
        member: &str,
        on: bool,
    ) -> Result<()> {
        if !valid_name(member) {
            return Err(Error::Invalid(format!(
                "bad member name: {}",
                shown(member)
            )));
        }
        self.administer(who, ("grant-rooms", &format!("{member}={on}")), |now| {
            self.store
                .policy_set(&format!("{GRANT}{member}"), on.then_some("yes"), now)?;
            Ok(String::new())
        })
    }

    /// Whether the member may create rooms and invite under the policy now in force.
    pub fn can_create_rooms(&self, member: &str) -> Result<bool> {
        if self.store.policy_get(POLICY)?.as_deref() != Some("granted") {
            return Ok(true);
        }
        Ok(self
            .store
            .policy_get(&format!("{GRANT}{member}"))?
            .is_some())
    }

    /// Refuse the two acts to a member without the capability; called for every operation by `check_role`.
    pub(super) fn check_room_capability(&self, who: &Principal, op: &Operation) -> Result<()> {
        let what = match op {
            Operation::CreateRoom { .. } => "create a room",
            Operation::Invite { .. } => "invite a member",
            _ => return Ok(()),
        };
        if self.can_create_rooms(&who.member)? {
            return Ok(());
        }
        Err(Error::Refused {
            rule: "room-capability".into(),
            reason: format!(
                "{} may not {what}; the person at the terminal grants that (klatch admin grant-rooms). \
                 For a need of two members, `klatch dm NAME TEXT` is free",
                who.member
            ),
        })
    }

    /// Whether `room` has never been used: no member and no message. A direct-message room is made by `dm`, which has
    /// its own rules, so it is never a creation here.
    fn is_new_room(&self, room: &str) -> Result<bool> {
        if self.dm_pair(room)?.is_some() {
            return Ok(false);
        }
        Ok(self.store.head(room)? == 0 && self.store.members(room)?.is_empty())
    }

    /// Before an act that would make `room`: ask the guard. Returns whether it would be a creation.
    pub(super) fn check_creation(&self, who: &Principal, room: &str) -> Result<bool> {
        if !self.is_new_room(room)? {
            return Ok(false);
        }
        self.authorize(who, &Operation::CreateRoom { room: room.into() })?;
        Ok(true)
    }

    /// After the act: keep who made the room.
    pub(super) fn note_creation(&self, who: &Principal, room: &str, created: bool) -> Result<()> {
        if !created {
            return Ok(());
        }
        self.open_room_grants(room, &who.member)?;
        let detail = format!("created by {}", who.member);
        self.store
            .record_audit(&NewAudit::new("create-room", room, &detail), self.now())
    }

    /// Refuse an invite to a room above the invitee's clearance (#189): the pointer would send the member where it is
    /// refused, and an inviter that cannot see clearances (the router) must not need to. Fails closed on an unknown label.
    fn check_invitee_clearance(&self, member: &str, room: &str) -> Result<()> {
        let chain = self.chain()?;
        let (label, held) = (self.room_label(room)?, self.clearance_in(member, room)?);
        if chain.dominates(&held, &label) {
            return Ok(());
        }
        let e = Error::Refused {
            rule: "clearance".into(),
            reason: format!(
                "room {room} is labelled {label}, above {member}'s clearance; no invite is posted"
            ),
        };
        Err(e)
    }

    /// Invite `member` to `room` by posting a pointer in `via`, a room the member is already in. It joins nobody; the
    /// invited session decides. Needs the capability and is audited with the inviter's name.
    pub fn invite(&self, who: &Principal, member: &str, room: &str, via: &str) -> Result<()> {
        check_member(who)?;
        for (what, name) in [("member", member), ("room", room), ("room", via)] {
            if !valid_name(name) {
                return Err(Error::Invalid(format!("bad {what} name: {}", shown(name))));
            }
        }
        if member == who.member {
            return Err(Error::Invalid("you cannot invite yourself".into()));
        }
        let op = Operation::Invite {
            member: member.into(),
            room: room.into(),
        };
        self.authorize(who, &op)?;
        self.check_invitee_clearance(member, room)
            .inspect_err(|e| self.note_refusal(who, (room, "invite"), e))?;
        if !self.who(who, via)?.iter().any(|m| m.name == member) {
            return Err(Error::Invalid(format!(
                "{member} is not in room {via}, so there is nowhere to reach them; name a room you share with --via"
            )));
        }
        self.post(who, via, &invite_text(&who.member, member, room))?;
        let detail = format!("invited by {} in {via}", who.member);
        self.store.record_audit(
            &NewAudit::new("invite", &format!("{member}@{room}"), &detail),
            self.now(),
        )
    }
}

/// The text of an invite: names only, one pointer, and no way for the sender to add words (#133).
pub fn invite_text(from: &str, member: &str, room: &str) -> String {
    format!(
        "invite: @{member}, {from} invites you to room {room}. To accept, run `klatch join {room}`; to ignore it, do nothing."
    )
}
