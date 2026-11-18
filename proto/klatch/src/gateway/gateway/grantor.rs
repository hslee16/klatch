//! Room-scoped clearance and the room grantor (#893, decision on #887, part of #180). Clearance is global per member
//! (`clearance:<member>`); a room grant (`room-clear:<room>:<member>`) lets one member read and post in one room up to
//! a label without raising its global clearance. The creator of a room is its initial grantor (`grantor:<room>`), and
//! the human names another or none (`klatch admin grantor`) and sets or revokes any grant (`klatch admin room-clear`).
//!
//! A grantor can give only what it holds, and the rules read nothing but the policy table and the roles, never a
//! message. A grant is refused unless every one of these holds, each its own rule so a refusal names the one that did:
//!
//! - `grantor`: the caller is the room's grantor now (a removed or replaced grantor grants nothing);
//! - `self-grant`: the grantee is not the grantor;
//! - `top-level`: the label is not the top of the chain;
//! - `agent-cap`: the label is no higher than the agent cap;
//! - `room-label`: the label is no higher than the room's label;
//! - `own-clearance`: the label is no higher than the grantor's own global clearance. Room grants are not passed on
//!   (not transitive): a grantor that is in a room only by a grant holds, as a grantor, its global clearance.
//!
//! The grantor must also pass the ordinary checks of the room (stops, its own clearance for the room) and its role
//! (a librarian or reviewer is refused). Every grant writes an audit row naming the grantor, grantee, room and
//! label; a refusal keeps a refusal row. A grant is read back only while it is a known label no higher than the
//! current agent cap, so lowering the cap or replacing the chain turns an old grant off (fail closed).

use super::policy::known_label;
use super::*;
use crate::store::NewAudit;

const GRANTOR: &str = "grantor:";
const GRANT: &str = "room-clear:";

impl<S: Store> Gateway<S> {
    /// The room's grantor, if it has one.
    pub fn room_grantor(&self, room: &str) -> Result<Option<String>> {
        self.store.policy_get(&format!("{GRANTOR}{room}"))
    }

    /// The label a member holds in one room by a grant, if the stored grant is still good: a label the chain has, no
    /// higher than the agent cap and not the top level. Anything else is as if there were no grant.
    fn room_grant_of(&self, member: &str, room: &str) -> Result<Option<String>> {
        let Some(label) = self.store.policy_get(&format!("{GRANT}{room}:{member}"))? else {
            return Ok(None);
        };
        let chain = self.chain()?;
        let cap = self.cap(&chain)?;
        let good = chain.rank(&label).is_some()
            && label != chain.highest()
            && chain.dominates(&cap, &label);
        Ok(good.then_some(label))
    }

    /// What a member holds in `room`: the higher of its global clearance and its grant for that room.
    pub(super) fn clearance_in(&self, member: &str, room: &str) -> Result<String> {
        let global = self.clearance_of(member)?;
        let Some(grant) = self.room_grant_of(member, room)? else {
            return Ok(global);
        };
        let chain = self.chain()?;
        Ok(if chain.dominates(&grant, &global) {
            grant
        } else {
            global
        })
    }

    /// After a room is made: its creator is the grantor, and nothing a room of that name once had carries over.
    pub(super) fn open_room_grants(&self, room: &str, creator: &str) -> Result<()> {
        let now = self.now();
        for (key, _) in self.store.policy_list(&format!("{GRANT}{room}:"))? {
            self.store.policy_set(&key, None, now)?;
        }
        self.store
            .policy_set(&format!("{GRANTOR}{room}"), Some(creator), now)
    }

    /// The room grantor grants `member` a clearance of `label` in `room` only, within the rules in the module doc.
    pub fn grant_room_clearance(
        &self,
        who: &Principal,
        room: &str,
        member: &str,
        label: &str,
    ) -> Result<()> {
        check_member(who)?;
        check_room(room)?;
        if !valid_name(member) {
            return Err(Error::Invalid(format!("bad member name: {member}")));
        }
        self.authorize(
            who,
            &Operation::GrantClearance {
                room: room.into(),
                member: member.into(),
            },
        )?;
        known_label(&self.chain()?, label)?;
        self.check_grant_rules(who, room, member, label)
            .inspect_err(|e| self.note_refusal(who, (room, "grant_clearance"), e))?;
        let now = self.now();
        self.store
            .policy_set(&format!("{GRANT}{room}:{member}"), Some(label), now)?;
        let detail = format!("{} granted {label} to {member} in {room}", who.member);
        self.store.record_audit(
            &NewAudit::new("grant-clearance", &format!("{room}/{member}"), &detail),
            now,
        )
    }

    fn check_grant_rules(
        &self,
        who: &Principal,
        room: &str,
        member: &str,
        label: &str,
    ) -> Result<()> {
        let refuse = |rule: &str, reason: String| Error::Refused {
            rule: rule.into(),
            reason,
        };
        let chain = self.chain()?;
        if self.room_grantor(room)?.as_deref() != Some(who.member.as_str()) {
            return Err(refuse(
                "grantor",
                format!(
                    "{} is not the grantor of room {room}; the person at the terminal names one (klatch admin grantor)",
                    who.member
                ),
            ));
        }
        if member == who.member {
            return Err(refuse(
                "self-grant",
                "a grantor cannot grant clearance to itself".into(),
            ));
        }
        if label == chain.highest() {
            return Err(refuse(
                "top-level",
                format!("{label} is the top level; no one is granted it"),
            ));
        }
        self.check_grantable(label)?;
        let room_label = self.room_label(room)?;
        if !chain.dominates(&room_label, label) {
            return Err(refuse(
                "room-label",
                format!("room {room} is labelled {room_label}; a grant there cannot be {label}"),
            ));
        }
        let own = self.clearance_of(&who.member)?;
        if !chain.dominates(&own, label) {
            return Err(refuse(
                "own-clearance",
                format!("you hold {own}; you cannot grant {label}"),
            ));
        }
        Ok(())
    }

    /// Only the human: name `member` the grantor of `room` (replacing any other), or with `on` false take the role
    /// from `member` (nothing changes if another member holds it). A floor act: no agent, the admin agent included.
    pub fn admin_set_room_grantor(
        &self,
        who: &Principal,
        room: &str,
        member: &str,
        on: bool,
    ) -> Result<()> {
        check_room(room)?;
        if !valid_name(member) {
            return Err(Error::Invalid(format!("bad member name: {member}")));
        }
        self.administer(who, ("grantor", &format!("{room}/{member}={on}")), |now| {
            let key = format!("{GRANTOR}{room}");
            if on {
                self.store.policy_set(&key, Some(member), now)?;
            } else if self.room_grantor(room)?.as_deref() == Some(member) {
                self.store.policy_set(&key, None, now)?;
            }
            Ok(String::new())
        })
    }

    /// Only the human: set `member`'s grant in `room` to `label` (at most the agent cap, not the top level), or with
    /// `None` revoke it. A floor act.
    pub fn admin_set_room_clearance(
        &self,
        who: &Principal,
        room: &str,
        member: &str,
        label: Option<&str>,
    ) -> Result<()> {
        check_room(room)?;
        if !valid_name(member) {
            return Err(Error::Invalid(format!("bad member name: {member}")));
        }
        let shown = label.unwrap_or("none");
        self.administer(
            who,
            ("room-clear", &format!("{room}/{member}={shown}")),
            |now| {
                if let Some(label) = label {
                    self.check_grantable(label)?;
                }
                self.store
                    .policy_set(&format!("{GRANT}{room}:{member}"), label, now)?;
                Ok(String::new())
            },
        )
    }
}
