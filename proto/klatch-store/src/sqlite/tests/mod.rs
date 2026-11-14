//! Tests of the SQLite store, by topic.

use super::*;

mod asks;
mod claims;
mod concurrency;
mod credentials;
mod encryption;
mod members;
mod messages;
mod migrations;
mod plaintext;
mod policy;

pub(super) fn store() -> SqliteStore {
    SqliteStore::open_in_memory().unwrap()
}

pub(super) fn deadlocked(r: Result<()>) -> Option<bool> {
    match r {
        Err(Error::Conflict { deadlock, .. }) => Some(deadlock),
        _ => None,
    }
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

pub(super) fn temp_db(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("klatch-store-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("rooms.db")
}

/// A plain connection to a file, unlocked with the test key: for building an old-schema database or holding a lock.
pub(super) fn keyed(path: &std::path::Path) -> Connection {
    let c = Connection::open(path).unwrap();
    c.pragma_update(
        None,
        "key",
        crate::insecure_test_key().sqlcipher_pragma().as_str(),
    )
    .unwrap();
    c
}
