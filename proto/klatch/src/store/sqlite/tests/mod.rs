//! Tests of the SQLite store, by topic.

use super::*;
use crate::store::areas::*;
use crate::store::{ClaimRequest, HookEvent};
use std::time::Duration;

mod anchors;
mod archive;
mod archive_files;
mod archive_rekey;
mod change_token;
mod claim_age;
mod claim_events;
mod claim_graph;
mod claims;
mod concurrency;
mod conformance;
mod credentials;
mod cycle_notice;
mod encryption;
mod label_keys;
mod long_tasks;
mod members;
mod messages;
mod migration_property;
mod migrations;
mod plaintext;
mod review;
mod runs;
mod shown;
mod stats;
mod wait_cycles;

pub(super) fn store() -> SqliteStore {
    SqliteStore::open_in_memory().unwrap()
}

pub(super) type Pairs = Vec<(String, String)>;

pub(super) fn changes(s: &SqliteStore, room: &str, member: &str, now: i64) -> (Pairs, Pairs) {
    let c = s.claim_changes(room, member, now).unwrap();
    (
        c.added.into_iter().map(|c| (c.owner, c.path)).collect(),
        c.released.into_iter().map(|c| (c.owner, c.path)).collect(),
    )
}

pub(super) fn pair(o: &str, p: &str) -> (String, String) {
    (o.to_string(), p.to_string())
}

pub(super) use crate::store::test_dir::{TempDb, temp_db};

/// A plain connection to a file, unlocked with the test key: for building an old-schema database or holding a lock.
pub(super) fn keyed(path: &std::path::Path) -> Connection {
    let c = Connection::open(path).unwrap();
    c.pragma_update(
        None,
        "key",
        crate::store::insecure_test_key()
            .sqlcipher_pragma()
            .as_str(),
    )
    .unwrap();
    c
}
