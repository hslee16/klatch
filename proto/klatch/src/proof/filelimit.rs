//! The failure limit for the acts that run when the store cannot be read (#1281): `admin recover` and `admin restore`.
//! The limit of #1233 lives in the store, which is unreadable then, so these attempts are kept in a file beside it
//! (`<store>.recovery-fails`): one line per charged attempt, the unix time. The wait is the same schedule as the
//! store's (`limit::lock_secs`), and an attempt is charged BEFORE the code is checked, by one append, so a process
//! killed after the code was typed, or several guessing at once, each still cost one.
//!
//! Once the store opens again, `settle` writes one audit row saying how many attempts were made and removes the
//! file. A file that is damaged (a line that is not a time) counts as an attempt made just now, so damage never
//! grants an attempt. Whoever can delete the file can already delete the store, so this guards the code against
//! guessing by a program that only has the command line, not against the owner of the files.

use super::limit;
use crate::store::NewAudit;
use crate::store::areas::PolicyStore;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The file of attempts for the store at `db`.
pub fn path_for(db: &Path) -> PathBuf {
    let mut name = db.as_os_str().to_os_string();
    name.push(".recovery-fails");
    PathBuf::from(name)
}

/// `(attempts so far, time of the last one)`; an unreadable file or line reads as an attempt at `now`.
fn read(db: &Path, now: i64) -> (u32, i64) {
    let text = match std::fs::read_to_string(path_for(db)) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (0, 0),
        Err(_) => return (u32::MAX, now),
    };
    let (mut count, mut last) = (0u32, 0i64);
    for line in text.lines() {
        count = count.saturating_add(1);
        last = last.max(line.trim().parse().unwrap_or(now));
    }
    (count, last)
}

/// Seconds the next attempt must wait, 0 when it may be made now.
pub fn wait_secs(db: &Path, now: i64) -> i64 {
    let (count, last) = read(db, now);
    limit::remaining(count, last, now)
}

/// Charge one attempt. `Err(secs)` means the wait is not over and nothing was charged; `Err` with a message in
/// the other arm means the file could not be written, which refuses the attempt (never grants one).
pub fn charge(db: &Path, now: i64) -> Result<(), Charge> {
    let wait = wait_secs(db, now);
    if wait > 0 {
        return Err(Charge::Wait(wait));
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path_for(db))
        .map_err(|e| Charge::Io(e.to_string()))?;
    f.write_all(format!("{now}\n").as_bytes())
        .map_err(|e| Charge::Io(e.to_string()))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Charge {
    Wait(i64),
    Io(String),
}

/// The store is readable again: record the attempts in its audit and forget them. Nothing to record is nothing done.
pub fn settle<S: PolicyStore + ?Sized>(db: &Path, store: &S, now: i64) {
    let (count, _) = read(db, now);
    if count == 0 {
        return;
    }
    let detail =
        format!("{count} attempts while the store could not be read (the last one was approved)");
    let _ = store.record_audit(&NewAudit::new("recovery-attempts", "", &detail), now);
    let _ = std::fs::remove_file(path_for(db));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> crate::store::test_dir::TestDir {
        crate::store::test_dir::TestDir::new("filelimit")
    }

    #[test]
    fn three_attempts_are_free_then_each_waits_longer() {
        let d = dir();
        let db = d.join("rooms.db");
        for _ in 0..limit::FREE {
            assert_eq!(charge(&db, 1000), Ok(()));
        }
        assert_eq!(charge(&db, 1000), Err(Charge::Wait(30)));
        assert_eq!(wait_secs(&db, 1010), 20);
        assert_eq!(charge(&db, 1030), Ok(()));
        assert_eq!(charge(&db, 1031), Err(Charge::Wait(59)));
    }

    #[test]
    fn a_damaged_file_never_grants_an_attempt() {
        let d = dir();
        let db = d.join("rooms.db");
        std::fs::write(path_for(&db), "1\n2\nnot a time\n").unwrap();
        assert!(wait_secs(&db, 5000) > 0);
    }

    #[test]
    fn an_unwritable_place_refuses_the_attempt() {
        let d = dir();
        let db = d.join("no-such-dir").join("rooms.db");
        assert!(matches!(charge(&db, 1), Err(Charge::Io(_))));
    }

    #[test]
    fn settle_writes_one_audit_row_and_forgets_the_attempts() {
        let d = dir();
        let db = d.join("rooms.db");
        charge(&db, 10).unwrap();
        charge(&db, 11).unwrap();
        let store = crate::store::SqliteStore::open_in_memory().unwrap();
        settle(&db, &store, 20);
        assert!(!path_for(&db).exists());
        assert!(
            store
                .last_audit_ts("recovery-attempts", "", 0)
                .unwrap()
                .is_some()
        );
        // nothing to settle writes nothing
        let other = crate::store::SqliteStore::open_in_memory().unwrap();
        settle(&db, &other, 30);
        assert!(
            other
                .last_audit_ts("recovery-attempts", "", 0)
                .unwrap()
                .is_none()
        );
    }
}
