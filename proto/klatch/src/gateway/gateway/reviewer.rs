//! The quarantined reviewer (#51, DESIGN D4): a member that reads one flagged message at a time and records a
//! verdict through the review queue, and can do nothing else. It has no tools and no write path to any room: every
//! operation but the two review ones is refused before the guard is asked, so no guard can be configured to let it
//! post, claim or change policy. It holds one role in the same `role:<member>` policy entry the librarian uses, so
//! a member is never two roles at once.

use super::*;
use crate::domain::{ReviewItem, Verdict};
use crate::gateway::DRAIN_LIMIT;
use crate::gateway::judge::{load_item, record};

const REVIEWER: &str = "reviewer";

impl<S: Store> Gateway<S> {
    /// Give a member the reviewer role, or take it away.
    pub fn admin_set_reviewer(&self, who: &Principal, member: &str, on: bool) -> Result<()> {
        self.admin_set_role(who, member, REVIEWER, on)
    }

    /// The member's role, if the human gave it one.
    pub(super) fn role_of(&self, member: &str) -> Result<Option<String>> {
        self.store.policy_get(&format!("role:{member}"))
    }

    /// Set a role, or clear it when it is the one the member holds; clearing a role the member does not hold
    /// leaves the one it has.
    pub(super) fn admin_set_role(
        &self,
        who: &Principal,
        member: &str,
        role: &str,
        on: bool,
    ) -> Result<()> {
        if !valid_name(member) {
            return Err(Error::Invalid(format!("bad member name: {member}")));
        }
        self.administer(who, (role, &format!("{member}={on}")), |now| {
            let held = self.role_of(member)?;
            if on || held.as_deref() == Some(role) {
                self.store
                    .policy_set(&format!("role:{member}"), on.then_some(role), now)?;
            }
            Ok(String::new())
        })
    }

    /// The role rules: a reviewer may only review, a librarian may only look, and only a reviewer may review.
    pub(super) fn check_role(&self, who: &Principal, op: &Operation) -> Result<()> {
        let role = self.role_of(&who.member)?;
        let refuse = |rule: &str, reason: String| {
            Err(Error::Refused {
                rule: rule.into(),
                reason,
            })
        };
        match (role.as_deref(), op.is_review()) {
            (Some(REVIEWER), true) => Ok(()),
            (Some(REVIEWER), false) => refuse(
                REVIEWER,
                format!("{} is a reviewer, which may only review", who.member),
            ),
            (_, true) => refuse(REVIEWER, format!("{} is not a reviewer", who.member)),
            (Some("librarian"), false) if !op.is_query() => refuse(
                "read-only",
                format!("{} is a librarian, which may only look", who.member),
            ),
            _ => Ok(()),
        }
    }

    /// The review operations' own entry: the member check, the role and guard, and a record of a refusal.
    fn admit_review(&self, who: &Principal, op: &Operation) -> Result<()> {
        check_member(who)?;
        self.check_guard(who, op).inspect_err(|e| {
            self.note_refusal(who, "", op.name(), e);
        })
    }

    /// The oldest waiting item the reviewer is cleared to read, with its text, or `None`. An item in a room above
    /// the reviewer's clearance is skipped, not shown: the human grants clearance to the reviewer like any member.
    pub fn review_next(&self, who: &Principal) -> Result<Option<ReviewItem>> {
        self.admit_review(who, &Operation::ReviewNext)?;
        for q in self.store.pending_reviews(DRAIN_LIMIT)? {
            if self.check_clearance(who, &q.room).is_err() {
                continue;
            }
            if let Some(item) = load_item(&self.store, &q)? {
                return Ok(Some(item));
            }
        }
        Ok(None)
    }

    /// Record the reviewer's verdict (allow, redact or deny) on a waiting item. It is final: only a waiting item
    /// takes a verdict, so a deny cannot be overridden, and a deferral is not a verdict.
    pub fn review_verdict(&self, who: &Principal, id: i64, verdict: &Verdict) -> Result<()> {
        self.admit_review(who, &Operation::ReviewVerdict { id })?;
        if *verdict == Verdict::Defer {
            return Err(Error::Invalid("a verdict is allow, redact or deny".into()));
        }
        let waiting = self.store.pending_reviews(usize::MAX >> 1)?;
        let Some(q) = waiting.into_iter().find(|q| q.id == id) else {
            return Err(Error::Invalid(format!("no waiting review {id}")));
        };
        self.check_clearance(who, &q.room)?;
        let why = match verdict {
            Verdict::Deny(why) => why.as_str(),
            _ => "",
        };
        let note = format!("by {}: {}", who.member, why);
        record(&self.store, &q, (verdict, &note), REVIEWER, self.now())
    }
}
