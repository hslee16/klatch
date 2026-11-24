//! The review, credential and policy parts of the spy store.

use super::*;

impl ReviewStore for SpyStore {
    fn enqueue_review(&self, item: &crate::store::NewReview, now: i64) -> Result<i64> {
        self.inner.enqueue_review(item, now)
    }

    fn pending_reviews(&self, limit: usize) -> Result<Vec<crate::store::QueuedReview>> {
        self.inner.pending_reviews(limit)
    }

    fn set_review_text(&self, id: i64, text: &str) -> Result<()> {
        self.inner.set_review_text(id, text)
    }

    fn reviews(&self, limit: usize) -> Result<Vec<crate::store::QueuedReview>> {
        self.inner.reviews(limit)
    }

    fn unapplied_reviews(&self, limit: usize) -> Result<Vec<crate::store::QueuedReview>> {
        self.inner.unapplied_reviews(limit)
    }

    fn mark_review_applied(&self, id: i64, outcome: &str) -> Result<()> {
        self.inner.mark_review_applied(id, outcome)
    }

    fn crossing_notices(
        &self,
        room: &str,
        member: &str,
    ) -> Result<Vec<crate::store::QueuedReview>> {
        self.inner.crossing_notices(room, member)
    }

    fn resolve_review(&self, resolution: &ReviewResolution, now: i64) -> Result<bool> {
        self.inner.resolve_review(resolution, now)
    }
}

impl CredentialStore for SpyStore {
    fn issue_credential(&self, c: &crate::store::NewCredential, now: i64) -> Result<()> {
        self.inner.issue_credential(c, now)
    }

    fn credential(&self, member: &str) -> Result<Option<crate::store::CredentialRecord>> {
        self.inner.credential(member)
    }

    fn revoke_credential(&self, member: &str, now: i64) -> Result<bool> {
        self.inner.revoke_credential(member, now)
    }

    fn renew_credential(&self, member: &str, now: i64, step: i64) -> Result<Option<i64>> {
        self.inner.renew_credential(member, now, step)
    }

    fn credentials(&self) -> Result<Vec<crate::store::CredentialRecord>> {
        self.inner.credentials()
    }
}

impl PolicyStore for SpyStore {
    fn policy_get(&self, key: &str) -> Result<Option<String>> {
        self.inner.policy_get(key)
    }

    fn policy_set(&self, key: &str, value: Option<&str>, now: i64) -> Result<()> {
        if let Some(hook) = self.before_policy_set.borrow().as_ref() {
            hook(key);
        }
        self.inner.policy_set(key, value, now)
    }

    fn reserved_policy_set(&self, key: &str, value: Option<&str>, now: i64) -> Result<()> {
        self.inner.reserved_policy_set(key, value, now)
    }

    fn mark_label_sealed_converting(&self, label: &str, now: i64) -> Result<usize> {
        if let Some(hook) = self.before_policy_set.borrow().as_ref() {
            hook(&format!("sealed:{label}"));
        }
        self.inner.mark_label_sealed_converting(label, now)
    }

    fn atomically(&self, act: &mut dyn FnMut() -> Result<()>) -> Result<()> {
        self.inner.atomically(act)
    }

    fn policy_list(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        self.inner.policy_list(prefix)
    }

    fn record_refusal(&self, entry: &crate::store::NewRefusal, now: i64) -> Result<()> {
        self.inner.record_refusal(entry, now)
    }

    fn refusals(&self, limit: usize) -> Result<Vec<crate::domain::RefusalRecord>> {
        self.inner.refusals(limit)
    }

    fn refusals_of(&self, member: &str, limit: usize) -> Result<Vec<crate::domain::RefusalRecord>> {
        self.inner.refusals_of(member, limit)
    }

    fn refusal_rules_since(&self, member: &str, since: i64) -> Result<Vec<String>> {
        self.inner.refusal_rules_since(member, since)
    }

    fn record_audit(&self, entry: &NewAudit, now: i64) -> Result<()> {
        self.inner.record_audit(entry, now)
    }

    fn audit_log(&self, limit: usize) -> Result<Vec<crate::domain::AuditRecord>> {
        self.inner.audit_log(limit)
    }

    fn last_audit_ts(&self, action: &str, target: &str, since: i64) -> Result<Option<i64>> {
        self.inner.last_audit_ts(action, target, since)
    }
}
