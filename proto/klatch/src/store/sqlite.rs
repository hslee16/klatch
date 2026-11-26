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
    /// Audit rows (#1592) that could not be written: counted, and kept queued for the next flush.
    audit_flush_errors: std::cell::Cell<u64>,
    /// Audit rows (#1275) of refused writes made inside a transaction, kept as `(action, target, ts)` until the
    /// outermost transaction has closed: a refusal rolls its own transaction back, and a row written inside would go
    /// with it. Names only, never a value.
    deferred_audit: std::cell::RefCell<Vec<(&'static str, String, i64)>>,
}

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
