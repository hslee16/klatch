//! The facilitator (#190, DESIGN section 24.1 decision 7): a member that routes asks into topical rooms, the
//! model-backed step after the rules router (#189). It holds one role in the `role:<member>` policy entry like every
//! other role (`roles.rs`), and only the human at a terminal grants it (`klatch admin facilitator`).
//!
//! The bound is an allowlist checked in `check_role`, before any `Guard` is asked, so no guard configuration can widen
//! it. A facilitator may:
//!
//! - look: `read` (with or without moving its own cursor), `who`, `claims`, the board and the open asks, all of them
//!   only in rooms at or below its own clearance (the lowest by default, like any member);
//! - join a room, which makes it when it is new (`create-room`, still subject to the room-creation capability, #188);
//! - invite a member (`invite`, which never goes above the invitee's clearance);
//! - post the invite pointer, and only that exact text: the sentence `invite` builds, which names a member and a room
//!   and carries no words of the sender's.
//!
//! Everything else is refused with the rule `facilitator`: any other post, claim, release, status, mute, ack, scope,
//! kind, direct message or crossing, every admin act (audit, prune and label order included) and the grant of room
//! clearance. It has no label call and no clearance call, and it never grants: a room it creates gets no grantor (its
//! creator is not made the grantor), and a grant by it is refused even when the human names it a room's grantor.

use super::roles::RoleChange;
use super::*;

impl<S: Store> Gateway<S> {
    /// Give a member the facilitator role, or take it away.
    pub fn admin_set_facilitator(&self, who: &Principal, member: &str, on: bool) -> Result<()> {
        self.set_role(
            who,
            RoleChange {
                member,
                role: Role::Facilitator,
                on,
            },
        )
    }

    /// Whether the member holds the facilitator role.
    pub fn is_facilitator(&self, member: &str) -> Result<bool> {
        Ok(self.role_of(member)? == Some(Role::Facilitator))
    }

    /// For the calls that act on the caller's own record and ask no guard (its scope and kind): a facilitator declares
    /// neither, so the bound has no gap.
    pub(super) fn refuse_facilitator(&self, who: &Principal, what: &str) -> Result<()> {
        if self.is_facilitator(&who.member)? {
            return Err(refused(who, what));
        }
        Ok(())
    }
}

fn refused(who: &Principal, what: &str) -> Error {
    Error::Refused {
        rule: "facilitator".into(),
        reason: format!(
            "{} is a facilitator, which may only read, create a room, invite and post the invite pointer; {what} is outside that",
            who.member
        ),
    }
}

/// The refusal of an operation a facilitator may not do, or `None` when it may.
pub(super) fn refusal(who: &Principal, op: &Operation) -> Option<Error> {
    let allowed = match op {
        Operation::Join { .. }
        | Operation::CreateRoom { .. }
        | Operation::Invite { .. }
        | Operation::Read { .. }
        | Operation::Who { .. }
        | Operation::Claims { .. }
        | Operation::Subscribe { .. } => true,
        Operation::Post { body, .. } => is_pointer(&who.member, body),
        _ => false,
    };
    (!allowed).then(|| refused(who, op.name()))
}

/// Whether `body` is exactly what `invite` posts for some member and room.
fn is_pointer(from: &str, body: &str) -> bool {
    let Some(rest) = body.strip_prefix("invite: @") else {
        return false;
    };
    let Some((member, rest)) = rest.split_once(", ") else {
        return false;
    };
    let Some(rest) = rest
        .strip_prefix(from)
        .and_then(|r| r.strip_prefix(" invites you to room "))
    else {
        return false;
    };
    let Some((room, _)) = rest.split_once(". To accept") else {
        return false;
    };
    valid_name(member) && valid_name(room) && super::rooms::invite_text(from, member, room) == body
}
