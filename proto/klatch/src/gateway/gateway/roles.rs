//! Roles (D4): one mechanism for every role. A member holds at most one, stored as the policy entry `role:<member>`,
//! and only the human at a terminal (`klatch admin`) grants or removes it; the role-holders themselves are refused
//! that act. `check_guard` is the one place every operation reaches the guard, so a role is enforced before any
//! `Guard` is asked and no guard configuration can widen it.
//!
//! - the librarian may only look (R30, rule `read-only`);
//! - the quarantined reviewer (#51) may only review: read the queue and record a verdict (rule `reviewer`), and only
//!   a reviewer may do that;
//! - the facilitator (#190) may only look, create a room, invite and post the invite pointer (rule `facilitator`);
//! - the admin agent (R30, #50) does the non-floor admin acts inside the floors: an act outside its list is refused
//!   (`admin-scope`), a floor act is refused (`floors`), and every refusal is audited.

use super::*;
use crate::store::NewAudit;

/// What a role-holder is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Librarian,
    Admin,
    Reviewer,
    Facilitator,
    Integrator,
}

/// A grant or removal of a role: whose, which, and whether it is given (`on`) or taken away.
#[derive(Clone, Copy)]
pub(super) struct RoleChange<'a> {
    pub member: &'a str,
    pub role: Role,
    pub on: bool,
}

impl Role {
    fn word(self) -> &'static str {
        match self {
            Role::Librarian => "librarian",
            Role::Admin => "admin",
            Role::Reviewer => "reviewer",
            Role::Facilitator => "facilitator",
            Role::Integrator => "integrator",
        }
    }

    fn parse(text: &str) -> Option<Role> {
        [
            Role::Librarian,
            Role::Admin,
            Role::Reviewer,
            Role::Facilitator,
            Role::Integrator,
        ]
        .into_iter()
        .find(|r| r.word() == text)
    }
}

/// The admin acts an admin agent may perform: rooms, members, clearances inside the cap, room labels, upkeep and
/// runs, and the records. Everything else is refused, including a stop of every room (the human's stop control),
/// roles, sealing, canaries, credentials and the floors. An allowlist, so a new admin act is out of scope until
/// someone decides it belongs here.
const ADMIN_AGENT_ACTS: [&str; 16] = [
    "close",
    "reopen",
    "remove",
    "allow",
    "clear",
    "label",
    "prune",
    "run-create",
    "run-end",
    "reap-anchors",
    "anchor-done",
    "audit",
    "refusals",
    "hook_runs",
    "label-order",
    "scrub-check",
];

impl<S: Store> Gateway<S> {
    /// Give `change.member` the role `change.role`, or take it away (only if it is the role it holds). Replaces any
    /// other role.
    pub(super) fn set_role(&self, who: &Principal, change: RoleChange) -> Result<()> {
        let RoleChange { member, role, on } = change;
        if !valid_name(member) {
            return Err(Error::Invalid(format!("bad member name: {member}")));
        }
        let held = self.role_of(member)?;
        self.administer(who, (role_action(role), &format!("{member}={on}")), |now| {
            let value = match (on, held) {
                (true, _) => Some(role.word()),
                (false, Some(h)) if h != role => Some(h.word()),
                (false, _) => None,
            };
            self.store
                .policy_set(&format!("role:{member}"), value, now)?;
            Ok(String::new())
        })
    }

    /// The member's role, if it has one.
    pub fn role_of(&self, member: &str) -> Result<Option<Role>> {
        Ok(self
            .store
            .policy_get(&format!("role:{member}"))?
            .as_deref()
            .and_then(Role::parse))
    }

    /// The caller's own role as its word (`integrator`, `reviewer`, ...), or `None` (#1090). A read of the caller's own
    /// policy entry only: no parameter names another member, and the answer is one fixed word, never text.
    pub fn my_role(&self, who: &Principal) -> Result<Option<String>> {
        check_member(who)?;
        self.check_own_record_live(who)?;
        Ok(self.role_of(&who.member)?.map(|r| r.word().to_string()))
    }

    /// Give a member the admin agent role, or take it away.
    pub fn admin_set_admin_agent(&self, who: &Principal, member: &str, on: bool) -> Result<()> {
        self.set_role(
            who,
            RoleChange {
                member,
                role: Role::Admin,
                on,
            },
        )
    }

    /// Whether the member holds the admin agent role.
    pub fn is_admin_agent(&self, member: &str) -> Result<bool> {
        Ok(self.role_of(member)? == Some(Role::Admin))
    }

    /// The one place every operation reaches the guard: the caller's role is enforced first.
    pub(super) fn check_guard(&self, who: &Principal, op: &Operation) -> Result<()> {
        self.check_credential_live(who)?;
        self.check_role(who, op)?;
        self.guard.check(who, op)
    }

    /// Verify explicitly, every time (R35, #161): a verified principal whose member holds an issued credential acts
    /// only while that credential is live. A session that outlives its credential (expired or revoked) is refused at
    /// the next request, not only at start. A principal with no credential record (proven some other way) and an
    /// unattributed one are not touched; a store failure refuses, never allows.
    fn check_credential_live(&self, who: &Principal) -> Result<()> {
        match self.credential_ended(who)? {
            Some(why) => Err(credential_refusal(why)),
            None if self.has_credential(who)? => self.renew_credential_on_use(&who.member),
            None => Ok(()),
        }
    }

    /// The same check for the methods that act only on the caller's own record (#835), where a verified principal
    /// whose credential ended is refused too. It renews nothing: a hook writing its own record is not a request that
    /// keeps a session alive. The few methods the end of a session needs do not call it (see AFTER_END in
    /// `tests/zero_trust.rs`).
    pub(super) fn check_own_record_live(&self, who: &Principal) -> Result<()> {
        match self.credential_ended(who)? {
            Some(why) => Err(credential_refusal(why)),
            None => Ok(()),
        }
    }

    /// The own-record check for a hook that proves the session is alive (#1027): a live credential is renewed, with
    /// the same bounds as a renewal on use (never past the cap, audited, throttled). Only the hook methods call it;
    /// a request from outside the hooks keeps nothing alive through it, and `SessionEnd` never does.
    pub(super) fn check_own_record_live_and_renew(&self, who: &Principal) -> Result<()> {
        self.check_own_record_live(who)?;
        if self.has_credential(who)? {
            self.renew_credential_on_use(&who.member)?;
        }
        Ok(())
    }

    /// Why the principal's credential no longer works, or `None` when it works or there is nothing to check (an
    /// unattributed principal, or a verified one with no credential record). A store failure is an error: it refuses.
    fn credential_ended(&self, who: &Principal) -> Result<Option<&'static str>> {
        if who.attribution != crate::domain::Attribution::Verified {
            return Ok(None);
        }
        let Some(rec) = self.store.credential(&who.member)? else {
            return Ok(None);
        };
        Ok(if rec.revoked {
            Some("the credential was revoked")
        } else if self.now() >= rec.expires {
            Some("the credential expired")
        } else {
            None
        })
    }

    fn has_credential(&self, who: &Principal) -> Result<bool> {
        Ok(who.attribution == crate::domain::Attribution::Verified
            && self.store.credential(&who.member)?.is_some())
    }

    /// A successful request renews its credential (#161): the expiry moves to now plus the idle timeout, never past
    /// the cap from issue. A renewal gains at least a tenth of the timeout (at most a minute) or reaches the cap, so a
    /// burst of requests writes and audits once. Each renewal is audited, with the new expiry and never the token.
    fn renew_credential_on_use(&self, member: &str) -> Result<()> {
        let now = self.now();
        let Some(rec) = self.store.credential(member)? else {
            return Ok(());
        };
        let step = (rec.idle / 10).clamp(1, 60);
        if let Some(expires) = self.store.renew_credential(member, now, step)? {
            self.store.record_audit(
                &crate::store::NewAudit::new(
                    "renew-credential",
                    member,
                    &format!("expires={expires} cap={}", rec.cap),
                ),
                now,
            )?;
        }
        Ok(())
    }

    pub(super) fn check_role(&self, who: &Principal, op: &Operation) -> Result<()> {
        self.check_reserved(who)?;
        let role = self.role_of(&who.member)?;
        if let Some(e) = reviewer_refusal(role, who, op) {
            return Err(e);
        }
        self.check_facilitator_bound(who, op, role)?;
        if let Some(e) = merge_claim_refusal(role, who, op) {
            return Err(e);
        }
        if self.for_members {
            self.check_member_admin(who, op, role)?;
        }
        self.check_room_capability(who, op)?;
        self.check_role_limit(who, op, role)
    }

    /// #523: on a members' gateway the system's own names are nobody's, so a typed one gets no standing.
    fn check_reserved(&self, who: &Principal) -> Result<()> {
        if self.for_members && crate::domain::is_reserved_name(&who.member) {
            return Err(Error::Refused {
                rule: "reserved-name".into(),
                reason: "that name is the system's and no member may act as it".into(),
            });
        }
        Ok(())
    }

    /// A facilitator's allowlist, then the state rules of its one DM; any other role is not held to either.
    fn check_facilitator_bound(
        &self,
        who: &Principal,
        op: &Operation,
        role: Option<Role>,
    ) -> Result<()> {
        if role != Some(Role::Facilitator) {
            return Ok(());
        }
        if let Some(e) = super::facilitator::refusal(who, op) {
            return Err(e);
        }
        if let Operation::Post { room, body } = op {
            self.check_facilitator_dm_post(who, room, body)?;
        }
        Ok(())
    }

    /// The limit of the role itself, last: a librarian only looks, the admin agent keeps to its list.
    fn check_role_limit(&self, who: &Principal, op: &Operation, role: Option<Role>) -> Result<()> {
        match role {
            Some(Role::Librarian) if !op.is_query() => Err(Error::Refused {
                rule: "read-only".into(),
                reason: format!("{} is a librarian, which may only look", who.member),
            }),
            Some(Role::Admin) => self.check_admin_scope(who, op),
            _ => Ok(()),
        }
    }

    /// On a members' gateway an admin act needs a role that has it: the admin agent (further limited by its list),
    /// or the librarian (whose own rule limits it to the records it may look at). Session-start housekeeping (`prune`) is every member's.
    /// A floor act is refused as a floor whoever asks.
    fn check_member_admin(
        &self,
        who: &Principal,
        op: &Operation,
        role: Option<Role>,
    ) -> Result<()> {
        let Operation::Admin { action, .. } = op else {
            return Ok(());
        };
        let floor = admin_agent_refusal(action).filter(|(rule, _)| *rule == "floors");
        // `hook_runs` (content-free, any member's) stays open until the human decides otherwise
        let allowed = matches!(
            action.as_str(),
            "prune" | "hook_runs" | "label-order" | "scrub-check"
        )
            || role == Some(Role::Admin)
            // the librarian arm of `check_role` refuses what is not a query, under its own rule
            || role == Some(Role::Librarian);
        let refusal = match (floor, allowed) {
            (Some(f), _) => f,
            (None, true) => return Ok(()),
            (None, false) => (
                "admin-scope",
                "that act is for the admin agent, not for a member",
            ),
        };
        self.refuse_admin(who, action, refusal)
    }

    /// An admin agent's act is refused unless it is on the list.
    fn check_admin_scope(&self, who: &Principal, op: &Operation) -> Result<()> {
        let Operation::Admin { action, .. } = op else {
            return Ok(());
        };
        match admin_agent_refusal(action) {
            Some(refusal) => self.refuse_admin(who, action, refusal),
            None => Ok(()),
        }
    }

    /// Refuse an admin act; the refusal is kept in the refusals and in the audit, never with text.
    fn refuse_admin(
        &self,
        who: &Principal,
        action: &str,
        (rule, reason): (&str, &str),
    ) -> Result<()> {
        let e = Error::Refused {
            rule: rule.into(),
            reason: reason.into(),
        };
        self.note_refusal(who, ("*", &format!("admin:{action}")), &e);
        let detail = format!("{} refused: {rule}", who.member);
        let _ = self
            .store
            .record_audit(&NewAudit::new("refused", action, &detail), self.now());
        Err(e)
    }
}

fn role_action(role: Role) -> &'static str {
    match role {
        Role::Librarian => "librarian",
        Role::Admin => "admin-agent",
        Role::Reviewer => "reviewer",
        Role::Facilitator => "facilitator",
        Role::Integrator => "integrator",
    }
}

/// The rule and reason for an act an admin agent may not do, or None when it may.
fn admin_agent_refusal(action: &str) -> Option<(&'static str, &'static str)> {
    if crate::gateway::guard::is_floor_act(action) {
        Some((
            "floors",
            "that edits a floor; floors are set by the person at a terminal, never by an agent",
        ))
    } else if !ADMIN_AGENT_ACTS.contains(&action) {
        Some(("admin-scope", "that act is outside the admin agent's role"))
    } else {
        None
    }
}

/// A reviewer may only review and only a reviewer may review; None when the operation is fine for the role.
fn reviewer_refusal(role: Option<Role>, who: &Principal, op: &Operation) -> Option<Error> {
    let reason = match (role == Some(Role::Reviewer), op.is_review()) {
        (true, false) => format!("{} is a reviewer, which may only review", who.member),
        (false, true) => format!("{} is not a reviewer", who.member),
        _ => return None,
    };
    Some(Error::Refused {
        rule: "reviewer".into(),
        reason,
    })
}

/// Only an integrator, or the human (the reserved name `admin`, which only the terminal path acts as), may take the
/// `merge:main` claim (#865). Any other holder of a role, the admin agent included, and any plain member is refused
/// with the rule `integrator` and the step that fixes it. None when the operation is not that claim or is allowed.
fn merge_claim_refusal(role: Option<Role>, who: &Principal, op: &Operation) -> Option<Error> {
    let Operation::Claim { path, .. } = op else {
        return None;
    };
    if !crate::domain::is_merge_claim(path) || may_take_merge_claim(role, who) {
        return None;
    }
    Some(Error::Refused {
        rule: "integrator".into(),
        reason: format!(
            "{} may not take merge:main, which only an integrator or the human may; ask the human to run \
             `klatch admin integrator {}`",
            who.member, who.member
        ),
    })
}

/// An integrator, or the human (the reserved name `admin`), may take `merge:main`.
fn may_take_merge_claim(role: Option<Role>, who: &Principal) -> bool {
    role == Some(Role::Integrator) || who.member == "admin"
}

fn credential_refusal(why: &str) -> Error {
    Error::Refused {
        rule: "credential".into(),
        reason: why.into(),
    }
}
