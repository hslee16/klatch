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
//! clearance. The one exception to "no direct message" is the need-info DM (#902), below. It has no label call and no clearance call, and it never grants: a room it creates gets no grantor (its
//! creator is not made the grantor), and a grant by it is refused even when the human names it a room's grantor.

use super::roles::RoleChange;
use super::*;
use crate::domain::DM_PREFIX;
use crate::store::NewAudit;

/// The reason codes of the one DM (#902): a small fixed set, so the message carries a code and not words.
pub const NEED_INFO_CODES: [&str; 5] = [
    "which-repo",
    "which-file",
    "which-goal",
    "which-deadline",
    "more-detail",
];
/// Asks live in the lobby.
const ASK_ROOM: &str = "lobby";

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

    /// The check of the one DM (#902) for a post by a facilitator into a DM room: the body is the fixed form (checked
    /// by `refusal` already) and the state must allow it. A non-facilitator is not held to it.
    pub(super) fn check_facilitator_dm_post(
        &self,
        who: &Principal,
        room: &str,
        body: &str,
    ) -> Result<()> {
        if !room.starts_with(DM_PREFIX) || parse_need_info(body).is_none() {
            return Ok(());
        }
        let Some((a, b)) = self.dm_pair(room)? else {
            return Err(refused(
                who,
                "a need-info message outside a direct message room",
            ));
        };
        let to = if a == who.member { b } else { a };
        self.check_need_info(who, NeedInfo::new(&to, room, body))
    }

    /// The state rules of the one DM: the ask is open, `to` asked it, and no earlier need-info for it from this
    /// facilitator in the room.
    fn check_need_info(&self, who: &Principal, dm: NeedInfo) -> Result<()> {
        let NeedInfo { to, room, body } = dm;
        let no = |why: String| Err(refused(who, &why));
        let Some((n, _)) = parse_need_info(body) else {
            return no("a message that is not the need-info form".into());
        };
        let asks = self.store.open_asks(ASK_ROOM, self.now())?;
        let Some(ask) = asks.iter().find(|a| a.seq == n) else {
            return no(format!("ask #{n} is not an open ask"));
        };
        if ask.sender != to {
            return no(format!(
                "{to} did not ask #{n}; only the asker may be asked"
            ));
        }
        // the prefix query sees a need-info at any depth of the room, not only in its oldest 5000 messages (#980)
        let sent = self.store.words(room, &["need-info #"])?;
        if sent.iter().any(|m| {
            m.sender == who.member && parse_need_info(&m.body).is_some_and(|(k, _)| k == n)
        }) {
            return no(format!(
                "one need-info message per ask, and #{n} has had its one"
            ));
        }
        Ok(())
    }

    /// Before a direct message by a facilitator makes its room: the form and the state rules, and the create-room
    /// capability when the room is new. A refusal here leaves no room behind. Returns whether the room is new.
    pub(super) fn check_facilitator_dm(&self, who: &Principal, dm: NeedInfo) -> Result<bool> {
        let NeedInfo { room, body, .. } = dm;
        let op = Operation::Post {
            room: room.into(),
            body: body.into(),
        };
        let gate = refusal(who, &op)
            .map_or(Ok(()), Err)
            .and_then(|()| self.check_need_info(who, dm));
        gate.inspect_err(|e| self.note_refusal(who, (room, "dm"), e))?;
        let new = self.dm_pair(room)?.is_none();
        if new {
            // not `authorize`: the room is not a DM room until `dm` records its pair, and `check_dm` reserves the name
            let op = Operation::CreateRoom { room: room.into() };
            self.check_guard(who, &op)
                .inspect_err(|e| self.note_refusal(who, (room, "create-room"), e))?;
        }
        Ok(new)
    }

    /// Audit a sent need-info message: who, which ask, which asker and the reason code, never any other text.
    pub(super) fn audit_facilitator_dm(
        &self,
        who: &Principal,
        dm: NeedInfo,
        created: bool,
    ) -> Result<()> {
        let NeedInfo { to, room, body } = dm;
        let Some((n, code)) = parse_need_info(body) else {
            return Ok(());
        };
        if created {
            let detail = format!("created by {} for a need-info message", who.member);
            self.store
                .record_audit(&NewAudit::new("create-room", room, &detail), self.now())?;
        }
        let detail = format!("{} asked {to} about ask #{n}: {code}", who.member);
        self.store
            .record_audit(&NewAudit::new("facilitator-dm", room, &detail), self.now())
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

/// The three parts of the one DM (#902): who is asked, the direct message room, and the need-info body.
#[derive(Clone, Copy)]
pub(super) struct NeedInfo<'a> {
    to: &'a str,
    room: &'a str,
    body: &'a str,
}

impl<'a> NeedInfo<'a> {
    pub(super) fn new(to: &'a str, room: &'a str, body: &'a str) -> Self {
        Self { to, room, body }
    }
}

fn refused(who: &Principal, what: &str) -> Error {
    Error::Refused {
        rule: "facilitator".into(),
        reason: format!(
            "{} is a facilitator, which may only read, create a room, invite, post the invite pointer and send one need-info message to an asker; {what} is outside that",
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
        Operation::Post { room, body } => {
            is_pointer(&who.member, body)
                || (room.starts_with(DM_PREFIX) && parse_need_info(body).is_some())
        }
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

/// The ask number and reason code of a need-info message, or `None` unless the body is exactly `need-info #N: CODE`:
/// N canonical digits, CODE one of `NEED_INFO_CODES`. Nothing else may follow; text after the code (the optional
/// field of #902, dropped in #905) makes the body no form.
pub fn parse_need_info(body: &str) -> Option<(i64, &str)> {
    let rest = body.strip_prefix("need-info #")?;
    let (digits, code) = rest.split_once(": ")?;
    let n: i64 = digits.parse().ok()?;
    if n < 1 || n.to_string() != digits {
        return None;
    }
    let code = NEED_INFO_CODES.iter().find(|c| **c == code)?;
    Some((n, code))
}
