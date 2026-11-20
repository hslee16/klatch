//! The `PolicyStore` area of the store seam; `Store` in the parent module combines every area.
//!
//! Every default here is an error (#485): policy decides labels, clearances, roles, floors and stops, the audit and
//! the refusals are the evidence and the breaker's input, so a backend that skips them must fail loudly, not answer
//! "no policy, nothing refused" and let every check pass.

use super::unsupported;
use crate::domain::{AuditRecord, Result};
use crate::store::{NewAudit, NewRefusal};

pub trait PolicyStore {
    /// Keep a record of an administrative act, and read the latest ones, newest first.
    fn record_audit(&self, _entry: &NewAudit, _now: i64) -> Result<()> {
        unsupported("an audit")
    }

    fn audit_log(&self, _limit: usize) -> Result<Vec<AuditRecord>> {
        unsupported("an audit")
    }

    /// The time of the newest audit entry of one action on one target at or after `since`, however far back it is
    /// (#1007): a filtered read, so a busy audit does not hide an old entry.
    fn last_audit_ts(&self, _action: &str, _target: &str, _since: i64) -> Result<Option<i64>> {
        unsupported("an audit")
    }

    /// Policy the administrator set: room labels, clearances, the label chain, scrub terms, tripwires (epic #9). One
    /// key and one value each; `None` clears a key.
    fn policy_get(&self, _key: &str) -> Result<Option<String>> {
        unsupported("policy")
    }

    fn policy_set(&self, _key: &str, _value: Option<&str>, _now: i64) -> Result<()> {
        unsupported("policy")
    }

    /// Run `act` as one write transaction (#897): every read and write it makes through this store is one atomic step,
    /// taken under the write lock, so a check made inside it still holds when its write lands. An error from `act`
    /// rolls everything back. The store's own single-statement writes made inside `act` join the transaction.
    fn atomically(&self, _act: &mut dyn FnMut() -> Result<()>) -> Result<()> {
        unsupported("a transaction")
    }

    /// Every policy entry whose key starts with `prefix`, in key order.
    fn policy_list(&self, _prefix: &str) -> Result<Vec<(String, String)>> {
        unsupported("policy")
    }

    /// Keep a record of a refused call, and read the latest ones (newest first) or the rules one member was refused
    /// under since a time.
    fn record_refusal(&self, _entry: &NewRefusal, _now: i64) -> Result<()> {
        unsupported("refusals")
    }

    fn refusals(&self, _limit: usize) -> Result<Vec<crate::domain::RefusalRecord>> {
        unsupported("refusals")
    }

    /// The latest `limit` refusals of one member, newest first: a member reads its own record, never another's.
    fn refusals_of(
        &self,
        _member: &str,
        _limit: usize,
    ) -> Result<Vec<crate::domain::RefusalRecord>> {
        unsupported("refusals")
    }

    fn refusal_rules_since(&self, _member: &str, _since: i64) -> Result<Vec<String>> {
        unsupported("refusals")
    }
}
