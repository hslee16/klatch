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

/// The most bytes of the file that are read (about 5000 attempts; the wait schedule makes that unreachable by
/// guessing, so a larger file is damage or an attack, and never a free attempt).
const MAX_FILE_BYTES: u64 = 64 * 1024;

/// Open the file without following a symlink and without blocking (a FIFO there cannot stall the open), so that
/// what is read and appended is the file itself, never what a link points at. `create` makes it when it is absent.
/// Opened for reading and appending: the check and the append go through one descriptor under one lock (`charge`).
fn open_plain(path: &Path, create: bool) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .append(create)
        .create(create)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
}

/// At most `MAX_FILE_BYTES` of an open file, and whether it held more. `None` when the descriptor is not a regular file.
fn read_capped(file: &mut std::fs::File) -> Option<(Vec<u8>, bool)> {
    use std::io::Read;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes).ok()?;
    let more = bytes.len() as u64 > MAX_FILE_BYTES;
    bytes.truncate(MAX_FILE_BYTES as usize);
    Some((bytes, more))
}

/// The attempts of an open file: an unreadable, irregular or oversized one reads as one attempt now and a count of
/// `u32::MAX`, so it is a wait and never a free attempt.
fn times_of(file: &mut std::fs::File, now: i64) -> (u32, Vec<i64>) {
    match read_capped(file) {
        Some((bytes, false)) => lines_to_times(&bytes, now),
        _ => (u32::MAX, vec![now]),
    }
}

/// The time of every attempt in the file, in file order (the last line is the newest attempt whatever the clock said);
/// a line that is not a time reads as an attempt at `now`, and a file that is not UTF-8 still has the lines it has.
/// A file that cannot be read safely (a symlink, a FIFO, one over `MAX_FILE_BYTES`) reads as `times_of` says.
fn read_times(db: &Path, now: i64) -> (u32, Vec<i64>) {
    match open_plain(&path_for(db), false) {
        Ok(mut f) => times_of(&mut f, now),
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

/// How many attempts are recorded for the store at `db` and not yet settled (`klatch doctor` shows it). A file that
/// cannot be read safely counts as `u32::MAX`.
pub fn attempts(db: &Path) -> u32 {
    read_times(db, 0).0
}

/// `(attempts so far, time of the last one)`.
#[cfg(test)]
fn read(db: &Path, now: i64) -> (u32, i64) {
    let (count, times) = read_times(db, now);
    (count, times.iter().copied().max().unwrap_or(0))
}

/// Seconds the next attempt must wait, 0 when it may be made now (`charge` decides under its lock; this is the read-only view).
#[cfg(test)]
pub fn wait_secs(db: &Path, now: i64) -> i64 {
    let (count, last) = read(db, now);
    limit::remaining(count, last, now)
}

/// Charge one attempt. `Err(secs)` means the wait is not over and nothing was charged; `Err` with a message in
/// the other arm means the file could not be written, which refuses the attempt (never grants one).
///
/// The check and the append are one step: the file is locked (`flock`, through std) from before it is read until the
/// attempt is appended, so callers racing at the moment one attempt is due cannot both pass (#1636). The lock is
/// advisory and held only by `charge`; whoever writes the file by hand is the owner, who can delete it anyway.
pub fn charge(db: &Path, now: i64) -> Result<(), Charge> {
    let io = |e: std::io::Error| Charge::Io(e.to_string());
    let mut f = open_plain(&path_for(db), true).map_err(io)?;
    f.lock().map_err(io)?;
    let (count, times) = times_of(&mut f, now);
    let wait = limit::remaining(count, times.iter().copied().max().unwrap_or(0), now);
    if wait > 0 {
        return Err(Charge::Wait(wait));
    }
    f.write_all(format!("{now}\n").as_bytes()).map_err(io)
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
    let read = open_plain(&settling, false)
        .ok()
        .and_then(|mut f| read_capped(&mut f))
        .map(|(bytes, _)| bytes)
        .ok_or(());
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

    #[test]
    fn a_symlink_is_neither_read_nor_appended_through() {
        let d = dir();
        let db = d.join("rooms.db");
        let target = d.join("elsewhere");
        std::fs::write(&target, "").unwrap();
        std::os::unix::fs::symlink(&target, path_for(&db)).unwrap();
        assert!(matches!(charge(&db, 1000), Err(Charge::Io(_))));
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"",
            "nothing written through the link"
        );
        assert!(
            wait_secs(&db, 1000) > 0,
            "a link reads as a wait, never a free attempt"
        );
    }

    #[test]
    fn a_file_over_the_cap_is_a_wait_not_a_read_of_all_of_it() {
        let d = dir();
        let db = d.join("rooms.db");
        std::fs::write(path_for(&db), "1\n".repeat(200_000)).unwrap();
        assert!(wait_secs(&db, 10_000_000) > 0);
        assert!(matches!(charge(&db, 10_000_000), Err(Charge::Wait(_))));
    }

    #[test]
    fn racing_callers_at_the_moment_one_attempt_is_due_get_exactly_one() {
        for round in 0..5 {
            let d = dir();
            let db = std::sync::Arc::new(d.join("rooms.db"));
            std::fs::write(path_for(&db), "1\n2\n").unwrap();
            let handles: Vec<_> = (0..32)
                .map(|_| {
                    let db = db.clone();
                    std::thread::spawn(move || charge(&db, 1000 + round).is_ok())
                })
                .collect();
            let passed = handles
                .into_iter()
                .filter(|_| true)
                .map(|h| h.join().unwrap())
                .filter(|&ok| ok)
                .count();
            assert_eq!(passed, 1, "round {round}");
        }
    }

    #[test]
    fn a_time_of_i64_max_in_the_file_is_a_wait_and_does_not_overflow() {
        let d = dir();
        let db = d.join("rooms.db");
        std::fs::write(path_for(&db), format!("1\n2\n3\n{}\n", i64::MAX)).unwrap();
        assert!(wait_secs(&db, 1000) > 0);
    }

    #[test]
    fn attempts_counts_the_file_for_the_doctor() {
        let d = dir();
        let db = d.join("rooms.db");
        assert_eq!(attempts(&db), 0);
        charge(&db, 10).unwrap();
        charge(&db, 11).unwrap();
        assert_eq!(attempts(&db), 2);
    }

    #[test]
    fn the_failure_note_names_the_count_and_the_file() {
        let d = dir();
        let db = d.join("rooms.db");
        charge(&db, 10).unwrap();
        charge(&db, 11).unwrap();
        let note = super::super::failed_attempt_note(&db);
        assert!(note.contains("2 recovery attempts"), "{note}");
        assert!(note.contains("recovery-fails"), "{note}");
        assert_eq!(super::super::recovery_attempts_recorded(&db), 2);
    }
}
