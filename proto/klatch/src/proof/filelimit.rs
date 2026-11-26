//! The failure limit for the acts that run when the store cannot be read (#1281): `admin recover` and `admin restore`.
//! The limit of #1233 lives in the store, which is unreadable then, so these attempts are kept in a file beside it
//! (`<store>.recovery-fails`): one line per charged attempt, the unix time. The wait is the same schedule as the
//! store's (`limit::lock_secs`), and an attempt is charged BEFORE the code is checked, by one append, so a process
//! killed after the code was typed, or several guessing at once, each still cost one.
//!
//! Once the store opens again, `settle` writes one audit row saying how many attempts were made, one `proof-failed`
//! row for each failed one (so the global alarm of #1310 counts them, #1552), and removes the file. A file that is damaged (a line that is not a time) counts as an attempt made just now, so damage never
//! grants an attempt. Whoever can delete the file can already delete the store, so this guards the code against
//! guessing by a program that only has the command line, not against the owner of the files.

use super::{alarm, limit};
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

/// The time of every attempt in the file, in file order (the last line is the newest attempt whatever the clock said);
/// a line that is not a time reads as an attempt at `now`, and a file that is not UTF-8 still has the lines it has.
/// An unreadable file reads as one attempt and a count of `u32::MAX`, so it is a wait and never a free attempt.
fn read_times(db: &Path, now: i64) -> (u32, Vec<i64>) {
    match std::fs::read(path_for(db)) {
        Ok(bytes) => lines_to_times(&bytes, now),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (0, Vec::new()),
        Err(_) => (u32::MAX, vec![now]),
    }
}

fn lines_to_times(bytes: &[u8], now: i64) -> (u32, Vec<i64>) {
    let times: Vec<i64> = String::from_utf8_lossy(bytes)
        .lines()
        .map(|l| l.trim().parse().unwrap_or(now))
        .collect();
    (u32::try_from(times.len()).unwrap_or(u32::MAX), times)
}

/// `(attempts so far, time of the last one)`.
fn read(db: &Path, now: i64) -> (u32, i64) {
    let (count, times) = read_times(db, now);
    (count, times.iter().copied().max().unwrap_or(0))
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

/// How far ahead of `now` a recorded time may be (clock skew) before it is dropped; a nearer one reads as `now`.
const SKEW_SECS: i64 = 300;

/// The store is readable again: record the attempts in its audit and forget them. Nothing to record is nothing done.
/// Every attempt but the last LINE was a failed guess at the recovery code (the last one is the one that was
/// approved), and each is written as a `proof-failed` row on the target `recovery`, stamped with its own time, so the
/// global alarm (#1310, #1552) counts it with the wrong passphrases. The row says only that it came through the file
/// limit: no code, no terminal label, no secret.
///
/// The file is plain and anything running as the owner can write it, so a time outside the last 24 hours (or more
/// than `SKEW_SECS` ahead) is dropped, never clamped, and the summary row counts how many were dropped; a time a
/// little ahead is clamped to `now`. At most `alarm::CEILING` of the newest failures are written, since more add
/// nothing to an alarm that fires at the ceiling. Settling is at most once: the file is renamed to a settling name
/// in its own directory and removed BEFORE any row is written, and when either step fails nothing is written. A
/// crash after that loses the counts not yet written; it never counts one twice.
pub fn settle<S: PolicyStore + ?Sized>(db: &Path, store: &S, now: i64) {
    let file = path_for(db);
    let mut name = file.as_os_str().to_os_string();
    name.push(".settling");
    let settling = PathBuf::from(name);
    if std::fs::rename(&file, &settling).is_err() {
        return;
    }
    let read = std::fs::read(&settling);
    if std::fs::remove_file(&settling).is_err() {
        return;
    }
    let Ok(bytes) = read else { return };
    let (count, times) = lines_to_times(&bytes, now);
    if count == 0 {
        return;
    }
    let failed = &times[..times.len() - 1];
    let mut kept: Vec<i64> = failed
        .iter()
        .filter(|&&t| {
            t > now.saturating_sub(alarm::WINDOW_SECS) && t <= now.saturating_add(SKEW_SECS)
        })
        .map(|&t| t.min(now))
        .collect();
    let dropped = failed.len() - kept.len();
    kept.sort_unstable();
    let skip = kept.len().saturating_sub(alarm::CEILING as usize);
    alarm::backfilled(
        store,
        "recovery",
        alarm::FILE_LIMIT_DETAIL,
        &kept[skip..],
        now,
    );
    let mut detail =
        format!("{count} attempts while the store could not be read (the last one was approved)");
    if dropped > 0 {
        detail.push_str(&format!(
            "; {dropped} attempts had a time outside the last 24 hours and were not counted as failures"
        ));
    }
    let _ = store.record_audit(&NewAudit::new("recovery-attempts", "", &detail), now);
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
