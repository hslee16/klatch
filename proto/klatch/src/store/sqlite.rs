use crate::domain::{ASK_LEASE_DEFAULT_SECS, CLAIM_DEFAULT_SECS};
use crate::domain::{Claim, ClaimChanges, Error, Message, Parked, Posted, ReleasedClaim, Result};
use crate::store::ids::new_id;
use crate::store::{NewMessage, Release, Seat, Seq, Shown};
use rusqlite::{Connection, params};

mod api;
mod archive;
mod archive_keys;
mod asks;
mod backup;
mod claim_chain;
mod claim_events;
mod claim_lapses;
mod claims;
mod connect;
mod credentials;
mod exec;
mod housekeeping;
mod integrity;
mod label_keys;
mod leases;
mod machine_checkpoint;
mod messaging;
mod parking;
mod releases;
mod review;
mod rows;
mod schema;
mod seal;
mod stats;
mod tasks;
pub use archive::{
    ArchiveInfo, ArchiveState, archives_to_rekey, list_archives, rekey_archives, rekey_archives_in,
};
pub use archive_keys::{reseal_archives, resealable_archives, wrap_archives};
use connect::be;
pub use integrity::check_store;
use rows::{MESSAGE_COLUMNS, attribution_text};
pub use schema::SCHEMA_VERSION;
use schema::WAIT_WINDOW_SECS;

/// SQLite in WAL mode. Many readers, one writer at a time; a claim check-and-insert and a post are each one
/// `BEGIN IMMEDIATE` transaction so they are atomic across processes.
///
/// Data-model rules (DESIGN D6): every row carries a `tenant` (always `local` in v1) and a `member`
/// (until the `Identity` trait exists the member id is the member's name); message ids are globally unique
/// and separate from the dense per-room `seq`.
pub struct SqliteStore {
    conn: Connection,
    /// The label keys this principal holds (DESIGN section 32 item 8). A label it has no key for stays sealed.
    labels: std::cell::RefCell<crate::keys::LabelKeys>,
    /// Claim events (#1386) that could not be recorded: the failure is swallowed so a claim never fails for it.
    claim_record_errors: std::cell::Cell<u64>,
    /// Lapse rows (#1586) dropped by the per-room and total caps (#1702), counted so a flood is not silent.
    claim_lapses_dropped: std::cell::Cell<u64>,
    /// Audit rows (#1592) that could not be written, one per row however many flushes it fails: kept queued.
    audit_flush_errors: std::cell::Cell<u64>,
    /// Audit rows (#1275) of refused writes made inside a transaction, kept as `(action, target, ts)` until the
    /// outermost transaction has closed: a refusal rolls its own transaction back, and a row written inside would go
    /// with it. Names only, never a value.
    deferred_audit: std::cell::RefCell<Vec<QueuedAudit>>,
    /// Refusal audit rows lost: (#1677) dropped from the full queue, oldest first, each also counted in
    /// `audit_flush_errors` as a failed row; and (#1701) refusals the throttle left unwritten, which are also in the
    /// minute's summary row (#1709).
    audit_rows_dropped: std::cell::Cell<u64>,
    /// Refusals the throttle left unwritten that are not yet in their minute's summary row (#1709): minute, count.
    throttled: std::cell::RefCell<std::collections::BTreeMap<i64, u64>>,
    /// ROLLBACKs that failed (#1678): the transaction they should have closed may still be open.
    rollback_errors: std::cell::Cell<u64>,
    /// Whether this store opened the transaction now open on the connection (#1678): `immediate` joins only that.
    owns_txn: std::cell::Cell<bool>,
    /// Whether that transaction is a `snapshot` (read only): a write inside it is an error.
    read_only_txn: std::cell::Cell<bool>,
    /// Test hook: how many of the next ROLLBACKs report failure without running.
    #[cfg(test)]
    fail_rollbacks: std::cell::Cell<u32>,
    /// Test hook: how many of the next summary row writes fail (#1709).
    #[cfg(test)]
    fail_summaries: std::cell::Cell<u32>,
}

/// A refusal audit row waiting for a flush: `(action, target, ts, failed)`; `failed` once a flush of it has failed,
/// so a row that fails N flushes is one failure (#1677).
type QueuedAudit = (&'static str, String, i64, bool);

fn credential_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<crate::store::CredentialRecord> {
    Ok(crate::store::CredentialRecord {
        member: r.get(0)?,
        token_hash: r.get(1)?,
        clearance: r.get(2)?,
        parent: r.get(3)?,
        issued: r.get(4)?,
        expires: r.get(5)?,
        revoked: r.get::<_, i64>(6)? != 0,
        idle: r.get(7)?,
        cap: r.get(8)?,
    })
}

#[cfg(test)]
mod tests;
