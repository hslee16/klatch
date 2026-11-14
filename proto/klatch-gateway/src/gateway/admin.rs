//! Administrative acts (`klatch admin`, R11): close or reopen a room, pause or resume every room, remove or allow a
//! member, and read the audit. Who may call them is decided by the caller; the gateway only asks the guard and keeps
//! a record of each act.

use super::*;
use klatch_store::NewAudit;

impl<S: Store> Gateway<S> {
    /// One administrative act: ask the guard, do it, and keep a record. Who may administer is decided by the
    /// caller (the CLI today: a real terminal with no agent session above it) and later by a guard and roles.
    pub(super) fn administer(
        &self,
        who: &Principal,
        (action, target): (&str, &str),
        act: impl FnOnce(i64) -> Result<String>,
    ) -> Result<()> {
        check_member(who)?;
        self.guard.check(
            who,
            &Operation::Admin {
                action: action.into(),
                target: target.into(),
            },
        )?;
        let now = self.now();
        let outcome = act(now)?;
        self.store
            .record_audit(&NewAudit::new(action, target, &outcome), now)
    }

    /// Make a room read-only, or open it again.
    pub fn admin_close_room(&self, who: &Principal, room: &str, closed: bool) -> Result<()> {
        check_room(room)?;
        let action = if closed { "close" } else { "reopen" };
        self.administer(who, (action, room), |now| {
            self.store
                .set_room_state(room, closed.then_some("closed"), now)?;
            Ok(String::new())
        })
    }

    /// Stop every room, or start them again.
    pub fn admin_pause(&self, who: &Principal, paused: bool) -> Result<()> {
        let action = if paused { "pause" } else { "resume" };
        self.administer(who, (action, "*"), |now| {
            self.store
                .set_room_state("*", paused.then_some("paused"), now)?;
            Ok(String::new())
        })
    }

    /// Take a member out of a room (`*` for every room it is in), release its claims there and refuse it there
    /// until it is allowed again.
    pub fn admin_remove_member(&self, who: &Principal, room: &str, member: &str) -> Result<()> {
        if room != "*" {
            check_room(room)?;
        }
        if !valid_name(member) {
            return Err(Error::Invalid(format!("bad member name: {member}")));
        }
        self.administer(who, ("remove", &format!("{room}/{member}")), |now| {
            let rooms = if room == "*" {
                self.store.rooms_of(member)?
            } else {
                vec![room.to_string()]
            };
            let mut released = 0;
            for r in &rooms {
                released += self.store.remove_member(r, member, now)?;
            }
            self.store.block_member(room, member, now)?;
            Ok(format!(
                "removed from {} room(s), {released} claim(s) released",
                rooms.len()
            ))
        })
    }

    /// Lift a removal.
    pub fn admin_allow_member(&self, who: &Principal, room: &str, member: &str) -> Result<()> {
        if room != "*" {
            check_room(room)?;
        }
        if !valid_name(member) {
            return Err(Error::Invalid(format!("bad member name: {member}")));
        }
        self.administer(who, ("allow", &format!("{room}/{member}")), |now| {
            let tripped = room == "*"
                && self
                    .store
                    .policy_get(&format!("trip:member:{member}"))?
                    .is_some();
            if tripped {
                self.store
                    .policy_set(&format!("trip:member:{member}"), None, now)?;
            }
            if self.store.unblock_member(room, member)? || tripped {
                Ok(String::new())
            } else {
                Err(Error::NotFound(format!(
                    "{member} is not removed from {room}"
                )))
            }
        })
    }

    /// Record an administrative act that was carried out outside the gateway (a key was shown, restored or rotated), so
    /// it is in the audit like every other. It asks the guard like any admin act.
    pub fn admin_note(
        &self,
        who: &Principal,
        (action, target): (&str, &str),
        outcome: &str,
    ) -> Result<()> {
        self.administer(who, (action, target), |_| Ok(outcome.to_string()))
    }

    /// The latest administrative acts, newest first.
    pub fn admin_audit(
        &self,
        who: &Principal,
        limit: usize,
    ) -> Result<Vec<klatch_core::AuditRecord>> {
        check_member(who)?;
        self.guard.check(
            who,
            &Operation::Admin {
                action: "audit".into(),
                target: String::new(),
            },
        )?;
        self.store.audit_log(limit)
    }

    /// The stops, before any policy: a member the breaker stopped or the admin removed, a paused gateway, and a room
    /// the admin closed or a tripwire paused (a closed room stays readable).
    pub(super) fn check_stops(&self, who: &Principal, room: &str, op: &Operation) -> Result<()> {
        let refuse = |rule: &str, reason: String| {
            Err(Error::Refused {
                rule: rule.into(),
                reason,
            })
        };
        if let Some(why) = self
            .store
            .policy_get(&format!("trip:member:{}", who.member))?
        {
            return refuse("breaker", why);
        }
        if self.store.is_blocked(room, &who.member)? {
            return refuse(
                "admin-block",
                format!("you were removed from room {room} by the admin"),
            );
        }
        if self.store.room_state("*")?.as_deref() == Some("paused") {
            return refuse("admin-pause", "rooms are paused by the admin".into());
        }
        if !op.changes_a_closed_room() {
            return Ok(());
        }
        match self.store.room_state(room)?.as_deref() {
            Some("closed") => refuse(
                "admin-close",
                format!("room {room} is closed by the admin; it is read-only"),
            ),
            Some("tripped") => refuse(
                "tripwire",
                format!(
                    "room {room} is paused by a tripwire; it is read-only until an admin reopens it"
                ),
            ),
            _ => Ok(()),
        }
    }
}
