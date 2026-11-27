//! The `PolicyStore` area of the store seam; `Store` in the parent module combines every area.
//!
//! Every default here is an error (#485): policy decides labels, clearances, roles, floors and stops, the audit and
//! the refusals are the evidence and the breaker's input, so a backend that skips them must fail loudly, not answer
//! "no policy, nothing refused" and let every check pass.

use super::unsupported;
use crate::domain::{AuditRecord, Result};
use crate::store::{NewAudit, NewRefusal};

/// What `PolicyStore::audit_census` counts.
pub struct Census<'a> {
    pub action: &'a str,
    pub targets: &'a [&'a str],
    pub since: i64,
    /// The last moment that counts (inclusive): a row stamped later is not counted.
    pub until: i64,
    pub cap: u64,
}

/// The policy keys the admin passphrase proof owns (#1275): the verifier, the failure counter, the unlock window,
/// anything else under the `admin-passphrase` prefix, and the `admin-unlock-` family (#1588). Only `PolicyStore::reserved_policy_set` writes them.
pub fn is_proof_key(key: &str) -> bool {
    let k = normalize_key(key);
    k.starts_with("admin-passphrase") || k.starts_with("admin-unlock-")
}

/// A key with surrounding whitespace (Unicode) removed and ASCII letters lowercased: the form the reserved-key
/// match, the refusal audit and its throttle use, so `ADMIN-PASSPHRASE` or ` admin-unlock-secs` is refused like the
/// exact key and cannot dodge the throttle by variant (#1588).
pub fn normalize_key(key: &str) -> String {
    key.trim().to_ascii_lowercase()
}

pub trait PolicyStore {
    /// Keep a record of an administrative act, and read the latest ones, newest first.
    fn record_audit(&self, _entry: &NewAudit, _now: i64) -> Result<()> {
        unsupported("an audit")
    }

    fn audit_log(&self, _limit: usize) -> Result<Vec<AuditRecord>> {
        unsupported("an audit")
    }

    /// The latest `limit` audit rows, newest first (#1490). `only: None` is the default view: every row except the
    /// operational `forwarded-ignored` note, with `hidden` the number of those notes in the audit. `only: Some(action)`
    /// is the rows of exactly that action, with `hidden` 0. The filter applies before the limit.
    fn audit_page(&self, _limit: usize, _only: Option<&str>) -> Result<crate::store::AuditPage> {
        unsupported("an audit")
    }

    /// The time of the newest audit entry of one action on one target at or after `since`, however far back it is
    /// (#1007): a filtered read, so a busy audit does not hide an old entry.
    fn last_audit_ts(&self, _action: &str, _target: &str, _since: i64) -> Result<Option<i64>> {
        unsupported("an audit")
    }

    /// How many entries of `q.action`, on any of `q.targets`, are at or after `q.since` and at or before `q.until`, counting no further than
    /// `q.cap`, and across how many distinct `terminal=` labels in the details of those counted (#1310): two numbers,
    /// never a label. It reads at most `q.cap` rows of each target through the `(action, target, ts)` index, so what
    /// it costs does not grow with the audit; `u64::MAX` is an exact count (a read that holds no write lock).
    fn audit_census(&self, _q: &Census) -> Result<(u64, u64)> {
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

    /// Write a key the admin passphrase proof owns (#1275): the verifier, the failure counter, the unlock window.
    /// Only `crate::proof` can call this: the `ProofCap` argument is a token only that module can build (#1588); `policy_set` refuses those keys, so no generic policy writer can replace the
    /// verifier, clear the counter or stretch the window. Any other key is refused here, so the two writers never
    /// overlap.
    fn reserved_policy_set(
        &self,
        _cap: &crate::proof::ProofCap,
        _key: &str,
        _value: Option<&str>,
        _now: i64,
    ) -> Result<()> {
        unsupported("policy")
    }

    /// Mark `label` sealed and protect every earlier plain message text of the label under the label's key, as one
    /// step (#378). Returns how many texts were converted. Refused, changing nothing, when there is something to
    /// convert and the label's key is not held.
    fn mark_label_sealed_converting(&self, _label: &str, _now: i64) -> Result<usize> {
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
