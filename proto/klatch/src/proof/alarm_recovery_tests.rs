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

/// Write `lines` verbatim as the attempts file and settle at `now`.
fn settle_lines(s: &SqliteStore, d: &crate::store::test_dir::TestDir, lines: &str, now: i64) {
    let db = d.join("rooms.db");
    std::fs::write(path_for(&db), lines).unwrap();
    settle(&db, s, now);
}

fn failed_times(s: &SqliteStore) -> Vec<i64> {
    let mut t: Vec<i64> = rows(s, "proof-failed").iter().map(|r| r.ts).collect();
    t.sort_unstable();
    t
}

fn summary(s: &SqliteStore) -> String {
    rows(s, "recovery-attempts")[0].detail.clone()
}

#[test]
fn times_outside_the_last_24_hours_are_dropped_not_clamped_and_counted_in_the_summary() {
    let (d, s) = (dir(), store());
    let now = 10 * WINDOW_SECS;
    let old = now - WINDOW_SECS - 1;
    let mut lines = String::new();
    for i in 0..CEIL {
        // 25 far in the future, 25 older than the window: none may count
        let t = if i % 2 == 0 {
            i64::MAX - i as i64
        } else {
            old - i as i64
        };
        lines.push_str(&format!("{t}\n"));
    }
    lines.push_str(&format!("{now}\n"));
    settle_lines(&s, &d, &lines, now);
    assert!(failed_times(&s).is_empty(), "{:?}", actions(&s));
    assert!(rows(&s, ACTION).is_empty());
    assert_eq!(recent(&s, now).unwrap().0, 0);
    let text = summary(&s);
    assert!(
        text.contains(&format!(
            "{CEIL} attempts had a time outside the last 24 hours"
        )),
        "{text}"
    );
    // a later real crossing still fires exactly at the ceiling
    for _ in 0..CEIL {
        super::alarm::failed(&s, "", "failures=1 terminal=ttys001/s1", now + 10);
    }
    assert_eq!(rows(&s, ACTION).len(), 1, "{:?}", actions(&s));
}

#[test]
fn a_time_a_little_in_the_future_is_clamped_to_now_and_a_far_one_is_dropped() {
    let (d, s) = (dir(), store());
    let now = 100_000;
    settle_lines(
        &s,
        &d,
        &format!(
            "{}\n{}\n{}\n{now}\n",
            now + 300,
            now + 301,
            now - WINDOW_SECS
        ),
        now,
    );
    // +300 is skew (clamped to now); +301 is dropped; exactly 24 h old is outside the window (dropped)
    assert_eq!(failed_times(&s), vec![now]);
    assert!(
        summary(&s).contains("2 attempts had a time outside"),
        "{}",
        summary(&s)
    );
}

#[test]
fn a_back_dated_row_does_not_count_every_later_row_toward_the_alarm() {
    let s = store();
    let t = 10 * WINDOW_SECS;
    for i in 0..30 {
        super::alarm::failed(&s, "", &format!("failures=1 terminal=ttys{i}/s{i}"), t);
    }
    // 21 rows stamped 40 hours earlier: each one's window holds only the back-dated rows, so no alarm
    for _ in 0..21 {
        super::alarm::failed(&s, "recovery", "failures=1 via=file-limit", t - 40 * 3600);
    }
    assert!(rows(&s, ACTION).is_empty(), "{:?}", actions(&s));
    assert_eq!(recent(&s, t).unwrap().0, 30);
}

#[cfg(unix)]
#[test]
fn an_unwritable_directory_settles_zero_rows_and_a_second_settlement_writes_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let (d, s) = (dir(), store());
    let sub = d.join("locked");
    std::fs::create_dir(&sub).unwrap();
    let db = sub.join("rooms.db");
    let text: String = (0..11).map(|i| format!("{}\n", 5000 + i)).collect();
    std::fs::write(path_for(&db), &text).unwrap();
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o555)).unwrap();
    settle(&db, &s, 5100);
    settle(&db, &s, 5100);
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(failed_times(&s).is_empty(), "{:?}", actions(&s));
    assert!(rows(&s, "recovery-attempts").is_empty());
    assert!(path_for(&db).exists(), "nothing was taken");
}

#[test]
fn a_second_settlement_writes_nothing_and_no_file_is_left() {
    let (d, s) = (dir(), store());
    let db = attempts(&d, 11, 5000);
    settle(&db, &s, 5000);
    settle(&db, &s, 5000);
    assert_eq!(failed_times(&s).len(), 10);
    assert_eq!(rows(&s, "recovery-attempts").len(), 1);
    assert!(!path_for(&db).exists());
    let mut settling = path_for(&db).into_os_string();
    settling.push(".settling");
    assert!(!std::path::Path::new(&settling).exists());
}

#[test]
fn a_file_that_is_not_utf8_counts_the_lines_it_has() {
    let (d, s) = (dir(), store());
    let db = d.join("rooms.db");
    std::fs::write(path_for(&db), b"\xff\xfe\n5000\n").unwrap();
    settle(&db, &s, 5000);
    let text = summary(&s);
    assert!(text.starts_with("2 attempts"), "{text}");
    assert_eq!(failed_times(&s), vec![5000]);
}

#[test]
fn the_smallest_time_never_panics_or_counts() {
    let (d, s) = (dir(), store());
    assert_eq!(super::alarm::since(i64::MIN), i64::MIN);
    super::alarm::failed(&s, "recovery", "failures=1 via=file-limit", i64::MIN);
    settle_lines(&s, &d, &format!("{}\n5000\n", i64::MIN), 5000);
    assert_eq!(recent(&s, 5000).unwrap().0, 0);
}

#[test]
fn the_last_line_is_the_approved_attempt_even_when_the_clock_went_back() {
    let (d, s) = (dir(), store());
    // the approved attempt was appended last but carries the smallest time
    settle_lines(&s, &d, "4000\n4500\n3000\n", 5000);
    assert_eq!(failed_times(&s), vec![4000, 4500]);
}

#[test]
fn the_newest_failures_are_kept_when_there_are_more_than_the_ceiling() {
    let (d, s) = (dir(), store());
    let n = CEIL + 7;
    let text: String = (0..=n).map(|i| format!("{}\n", 1000 + i)).collect();
    settle_lines(&s, &d, &text, 1000 + n as i64);
    let t = failed_times(&s);
    assert_eq!(t.len(), CEIL);
    assert_eq!(*t.first().unwrap(), 1000 + 7);
    assert_eq!(*t.last().unwrap(), 1000 + n as i64 - 1);
}
