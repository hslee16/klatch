//! The quarantined reviewer (#51, DESIGN D4): a member that reads one flagged message at a time and records a
//! verdict through the review queue, and can do nothing else. It has no tools and no write path to any room: every
//! operation but the two review ones is refused before the guard is asked, so no guard can be configured to let it
//! post, claim or change policy. It holds one role in the same `role:<member>` policy entry the librarian uses, so
//! a member is never two roles at once (`roles.rs`).

use super::roles::RoleChange;
use super::*;
use crate::domain::{ReviewItem, Verdict};
use crate::gateway::judge::{PendingPages, load_item, record};
use crate::store::QueuedReview;

const REVIEWER: &str = "reviewer";

impl<S: Store> Gateway<S> {
    /// Give a member the reviewer role, or take it away.
    pub fn admin_set_reviewer(&self, who: &Principal, member: &str, on: bool) -> Result<()> {
        self.set_role(
            who,
            RoleChange {
                member,
                role: Role::Reviewer,
                on,
            },
        )
    }

    /// The review operations' own entry: the member check, the role and guard, and a record of a refusal.
    fn admit_review(&self, who: &Principal, op: &Operation) -> Result<()> {
        check_member(who)?;
        self.check_guard(who, op).inspect_err(|e| {
            self.note_refusal(who, ("", op.name()), e);
        })
    }

    /// The oldest waiting item the reviewer is cleared to read, with its text, or `None`. An item in a room above
    /// the reviewer's clearance is skipped, not shown: the human grants clearance to the reviewer like any member.
    pub fn review_next(&self, who: &Principal) -> Result<Option<ReviewItem>> {
        self.admit_review(who, &Operation::ReviewNext)?;
        let mut pages = PendingPages::new();
        while let Some(page) = pages.next(&self.store)? {
            for q in page {
                if self.check_clearance(who, &q.room).is_err() || !self.may_see_target(who, &q) {
                    continue;
                }
                if let Some(mut item) = load_item(&self.store, &q)? {
                    if !q.detail.is_empty() {
                        // why a judge flagged it (the rules, or the hosted judge): plain data, not an instruction to the reviewer
                        item.reason = format!("{}; judge flag (data): {}", item.reason, q.detail);
                    }
                    return Ok(Some(item));
                }
            }
        }
        Ok(None)
    }

    /// Whether the reviewer may be told where an item would be released: a plain item has no target room, and a
    /// crossing's target must pass the same clearance check as the source. An item whose target the reviewer cannot
    /// read is treated like one whose text it cannot read (skipped, its room name never shown), so the target room
    /// is no side channel to a room above the reviewer's clearance (#1751).
    fn may_see_target(&self, who: &Principal, q: &QueuedReview) -> bool {
        q.to_room.is_empty() || self.check_clearance(who, &q.to_room).is_ok()
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
        if !self.may_see_target(who, &q) {
            return Err(self.check_clearance(who, &q.to_room).unwrap_err());
        }
        let why = match verdict {
            Verdict::Deny(why) => why.as_str(),
            _ => "",
        };
        let note = format!("by {}: {}", who.member, why);
        record(&self.store, &q, (verdict, &note), (REVIEWER, self.now()))?;
        self.release_pending(&who.member).map(|_| ())
    }
}
