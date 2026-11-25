//! Failed recoveries through the file limit (#1552) count toward the same alarm as wrong passphrases (#1310).
//! `recover` and `restore` of an unreadable store keep their attempts in a file; when the store opens again,
//! `settle` turns every attempt but the approved last one into a `proof-failed` row on the target `recovery`, so
//! the one census, the one threshold and the one alarm row cover them. Times are unix seconds the test chooses.

use super::alarm::{ACTION, CEILING, WINDOW_SECS, recent};
use super::filelimit::{path_for, settle};
use super::tests::{actions, everything_kept, store, with_pass};
use super::*;
use crate::store::SqliteStore;

const CEIL: usize = CEILING as usize;

/// A file of `n` attempts, one a second ending at `last`: the last was the approved one, the others failed.
fn attempts(d: &crate::store::test_dir::TestDir, n: usize, last: i64) -> std::path::PathBuf {
    let db = d.join("rooms.db");
    let lines: String = (0..n)
        .map(|i| format!("{}\n", last - (n - 1 - i) as i64))
        .collect();
    std::fs::write(path_for(&db), lines).unwrap();
    db
}

fn dir() -> crate::store::test_dir::TestDir {
    crate::store::test_dir::TestDir::new("alarm-recovery")
}

fn rows(s: &SqliteStore, action: &str) -> Vec<crate::domain::AuditRecord> {
    s.audit_log(10_000)
        .unwrap()
        .into_iter()
        .filter(|a| a.action == action)
        .collect()
}

#[test]
fn fifty_failed_recoveries_in_the_window_raise_the_alarm_exactly_once() {
    let (d, s) = (dir(), store());
    // 51 attempts: 50 failed and the approved last one
    let db = attempts(&d, CEIL + 1, 5000);
    settle(&db, &s, 5000);
    let alarm = rows(&s, ACTION);
    assert_eq!(alarm.len(), 1, "{:?}", actions(&s));
    assert_eq!(alarm[0].target, "alarm");
    assert_eq!(
        alarm[0].detail,
        format!("failures={CEIL} terminals=0 window=24h")
    );
    assert_eq!(recent(&s, 5000).unwrap().0, CEILING);
    // and one summary row, as before
    assert_eq!(rows(&s, "recovery-attempts").len(), 1);
}

#[test]
fn one_failure_fewer_writes_no_alarm_and_a_second_settlement_past_it_writes_none_again() {
    let (d, s) = (dir(), store());
    let db = attempts(&d, CEIL, 5000);
    settle(&db, &s, 5000);
    assert!(rows(&s, ACTION).is_empty(), "{:?}", actions(&s));
    assert_eq!(recent(&s, 5000).unwrap().0, CEILING - 1);
    // one more failure from a later settlement crosses; a further one adds nothing
    let db = attempts(&d, 3, 5100);
    settle(&db, &s, 5100);
    assert_eq!(rows(&s, ACTION).len(), 1, "{:?}", actions(&s));
    let db = attempts(&d, 3, 5200);
    settle(&db, &s, 5200);
    assert_eq!(rows(&s, ACTION).len(), 1);
}

#[test]
fn a_successful_recovery_is_not_counted() {
    let (d, s) = (dir(), store());
    // only the approved attempt: nothing failed
    let db = attempts(&d, 1, 5000);
    settle(&db, &s, 5000);
    assert_eq!(recent(&s, 5000).unwrap(), (0, 0));
    assert!(rows(&s, "proof-failed").is_empty());
    assert_eq!(rows(&s, "recovery-attempts").len(), 1);
}

#[test]
fn recovery_failures_add_up_with_passphrase_failures() {
    let (d, s) = (dir(), store());
    with_pass(&s);
    for i in 0..30 {
        super::alarm::failed(&s, "", &format!("failures=1 terminal=ttys{i}/s{i}"), 4000);
    }
    assert!(rows(&s, ACTION).is_empty());
    // 20 failed recoveries (21 attempts) bring the sum to 50
    let db = attempts(&d, 21, 5000);
    settle(&db, &s, 5000);
    assert_eq!(rows(&s, ACTION).len(), 1, "{:?}", actions(&s));
    let alarm = &rows(&s, ACTION)[0];
    assert_eq!(alarm.detail, "failures=50 terminals=30 window=24h");
}

#[test]
fn attempts_older_than_the_window_are_not_counted() {
    let (d, s) = (dir(), store());
    let now = 10 * WINDOW_SECS;
    let db = attempts(&d, CEIL + 1, now - WINDOW_SECS - 100);
    settle(&db, &s, now);
    assert_eq!(recent(&s, now).unwrap().0, 0);
    // the burst was itself 50 inside one day, so its own moment still records the crossing, stamped then
    assert!(rows(&s, ACTION).iter().all(|a| a.ts < now - WINDOW_SECS));
}

#[test]
fn a_file_of_many_attempts_writes_at_most_a_ceiling_of_rows() {
    let (d, s) = (dir(), store());
    let db = attempts(&d, 5 * CEIL, 5000);
    settle(&db, &s, 5000);
    assert_eq!(rows(&s, "proof-failed").len(), CEIL);
    assert_eq!(rows(&s, ACTION).len(), 1);
    let summary = &rows(&s, "recovery-attempts")[0];
    assert!(
        summary
            .detail
            .starts_with(&format!("{} attempts", 5 * CEIL))
    );
}

#[test]
fn the_rows_carry_counts_only_and_no_code_passphrase_or_verifier() {
    let (d, s) = (dir(), store());
    with_pass(&s);
    let db = attempts(&d, 6, 5000);
    settle(&db, &s, 5000);
    let failed = rows(&s, "proof-failed");
    assert_eq!(failed.len(), 5);
    for r in &failed {
        assert_eq!(r.target, "recovery");
        assert_eq!(r.detail, "failures=1 via=file-limit");
    }
    let kept = everything_kept(&s);
    assert!(
        !kept.contains("KLATCH-") && !kept.contains("terminal="),
        "{kept}"
    );
}

#[test]
fn a_damaged_file_still_settles_and_never_writes_more_than_the_cap() {
    let (d, s) = (dir(), store());
    let db = d.join("rooms.db");
    std::fs::write(path_for(&db), "1\nnot a time\n7\n").unwrap();
    settle(&db, &s, 9000);
    // 3 attempts, 2 failed; the damaged line reads as an attempt made now
    assert_eq!(rows(&s, "proof-failed").len(), 2);
    assert!(!path_for(&db).exists());
}
