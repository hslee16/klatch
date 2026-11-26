//! The global alarm on failed proofs (#1310): counts across all terminals in a rolling 24 hours, one row per
//! crossing, never a lockout. Times are unix seconds the test chooses, never a calendar date.

use super::alarm::{ACTION, CEILING, WINDOW_SECS, recent};
use super::tests::{PASS, Typed, actions, at, everything_kept, store, with_pass};
use super::*;
use crate::store::SqliteStore;

const CEIL: usize = CEILING as usize;

/// One wrong passphrase typed on terminal `tty` at time `t`.
fn fail_on(s: &SqliteStore, tty: &'static str, t: i64) {
    let e = prove(
        s,
        &mut Typed::new(&["nope nope nope"]).on(tty),
        at(t),
        &Unlock::new(),
        "a test",
    )
    .unwrap_err();
    assert_eq!(e, ProofError::Wrong);
}

/// The terminal labels of a program that opens a new one for every three guesses (the free ones), `n` of them.
fn labels(n: usize) -> Vec<&'static str> {
    (0..n)
        .map(|i| &*Box::leak(format!("ttys{i:03}/s{i}").into_boxed_str()))
        .collect()
}

/// `n` failures at time `t`, three on each of fresh terminals starting at terminal number `first`.
fn fail_n(s: &SqliteStore, n: usize, first: usize, t: i64) {
    let tty = labels(first + n.div_ceil(3) + 1);
    for i in 0..n {
        fail_on(s, tty[first + i / 3], t);
    }
}

fn alarms(s: &SqliteStore) -> Vec<crate::domain::AuditRecord> {
    s.audit_log(10_000)
        .unwrap()
        .into_iter()
        .filter(|a| a.action == ACTION)
        .collect()
}

#[test]
fn one_failure_fewer_than_the_ceiling_across_terminals_writes_no_row() {
    let s = store();
    with_pass(&s);
    fail_n(&s, CEIL - 1, 0, 1000);
    assert!(alarms(&s).is_empty(), "{:?}", actions(&s));
    let (n, terms) = recent(&s, 1000).unwrap();
    assert_eq!(n, CEILING - 1);
    assert_eq!(terms as usize, (CEIL - 1).div_ceil(3));
}

#[test]
fn the_failure_that_reaches_the_ceiling_writes_exactly_one_loud_row_of_counts_only() {
    let s = store();
    with_pass(&s);
    fail_n(&s, CEIL, 0, 1000);
    let rows = alarms(&s);
    assert_eq!(rows.len(), 1, "{:?}", actions(&s));
    let terms = CEIL.div_ceil(3);
    assert_eq!(rows[0].target, "alarm");
    assert_eq!(
        rows[0].detail,
        format!("failures={CEIL} terminals={terms} window=24h")
    );
    assert_eq!(rows[0].ts, 1000);
    assert!(
        !rows[0].detail.contains("ttys") && !rows[0].detail.contains("nope"),
        "{:?}",
        rows[0]
    );
    // it is written after the failure that crossed, so the audit shows the failure first
    let all = actions(&s);
    assert_eq!(all[0], ACTION, "newest first: {all:?}");
    assert_eq!(all[1], "proof-failed");
}

#[test]
fn failures_after_the_ceiling_write_no_second_row() {
    let s = store();
    with_pass(&s);
    fail_n(&s, CEIL + 20, 0, 1000);
    assert_eq!(alarms(&s).len(), 1, "{:?}", actions(&s));
}

#[test]
fn the_window_is_a_rolling_24_hours_and_a_failure_exactly_that_old_no_longer_counts() {
    let s = store();
    with_pass(&s);
    fail_on(&s, "ttys001/s1", 5000);
    assert_eq!(recent(&s, 5000 + WINDOW_SECS - 1).unwrap().0, 1);
    assert_eq!(recent(&s, 5000 + WINDOW_SECS).unwrap().0, 0);
}

#[test]
fn the_alarm_rearms_once_the_failures_roll_out_of_the_window() {
    let s = store();
    with_pass(&s);
    fail_n(&s, CEIL, 0, 1000);
    assert_eq!(alarms(&s).len(), 1);
    // just before the old ones leave the window the count is still above the line: one more writes nothing
    fail_n(&s, 1, 100, 1000 + WINDOW_SECS - 1);
    assert_eq!(alarms(&s).len(), 1);
    // all of them have rolled out: the count starts again, and the ceiling-th failure writes the second row
    let later = 1000 + 2 * WINDOW_SECS;
    fail_n(&s, CEIL - 1, 200, later);
    assert_eq!(alarms(&s).len(), 1, "one short of the ceiling again");
    fail_n(&s, 1, 400, later);
    let rows = alarms(&s);
    assert_eq!(rows.len(), 2, "{:?}", actions(&s));
    assert_eq!(rows[0].ts, later);
}

#[test]
fn a_success_does_not_reset_the_count() {
    let s = store();
    with_pass(&s);
    fail_n(&s, CEIL - 1, 0, 1000);
    prove(
        &s,
        &mut Typed::new(&[PASS]).on("ttys900/s900"),
        at(1001),
        &Unlock::new(),
        "a test",
    )
    .unwrap();
    assert_eq!(recent(&s, 1002).unwrap().0, CEILING - 1);
    fail_n(&s, 1, 500, 1002);
    assert_eq!(alarms(&s).len(), 1, "{:?}", actions(&s));
}

#[test]
fn a_wrong_recovery_code_counts_like_a_wrong_passphrase() {
    let s = store();
    with_pass(&s);
    fail_n(&s, CEIL - 1, 0, 1000);
    let e = reset(
        &s,
        "KLATCH-AAAA-AAAA",
        &mut Typed::new(&["KLATCH-BBBB-BBBB"]).on("ttys950/s950"),
        at(1001),
        &Unlock::new(),
    )
    .unwrap_err();
    assert_eq!(e, ProofError::Wrong);
    assert_eq!(alarms(&s).len(), 1, "{:?}", actions(&s));
}

#[test]
fn the_alarm_locks_nothing_and_changes_no_counter_and_adds_no_policy_key() {
    let s = store();
    with_pass(&s);
    let keys_before: Vec<String> = s
        .policy_list("")
        .unwrap()
        .into_iter()
        .map(|e| e.0)
        .collect();
    fail_n(&s, CEIL, 0, 1000);
    assert_eq!(alarms(&s).len(), 1);
    let mut keys: Vec<String> = s
        .policy_list("")
        .unwrap()
        .into_iter()
        .map(|e| e.0)
        .collect();
    keys.sort();
    let mut expect = keys_before;
    expect.push(limit::KEY.to_string());
    expect.sort();
    assert_eq!(keys, expect, "the alarm adds no policy key");
    // a terminal that failed three times is still locked, and a new one has no wait at all
    let mut locked = Typed::new(&[PASS]).on("ttys000/s0");
    let e = prove(&s, &mut locked, at(1001), &Unlock::new(), "a test").unwrap_err();
    assert!(matches!(e, ProofError::Locked(w) if w > 0), "{e:?}");
    assert_eq!(locked.asked, 0);
    let mut fresh = Typed::new(&[PASS]).on("ttys777/s777");
    prove(&s, &mut fresh, at(1001), &Unlock::new(), "a test").unwrap();
    assert_eq!(fresh.asked, 1);
}

#[test]
fn nothing_typed_is_in_the_audit_after_a_crossing() {
    let s = store();
    with_pass(&s);
    fail_n(&s, CEIL, 0, 1000);
    let kept = everything_kept(&s);
    assert!(!kept.contains("nope nope nope"), "{kept}");
    assert!(!kept.contains(PASS), "{kept}");
}

#[test]
fn only_the_proof_path_names_the_alarm_action_or_the_failure_action() {
    // An agent reaches the audit only through commands that name their own actions. The alarm row is written by
    // `alarm.rs`, and its input, the `proof-failed` rows, by the proof path: no other source may name either (as the
    // text or through the constants), or it could forge the alarm or, with 50 forged failures, suppress it for 24
    // hours. The doctor reads the count through `alarm::read_recent` and names neither.
    const NAMES: [&str; 5] = [
        "proof-failed",
        "alarm::ACTION",
        "alarm::FAILED",
        "alarm::{",
        "alarm::*",
    ];
    const ALLOWED: [&str; 6] = [
        "proof/alarm.rs",
        "proof/alarm_recovery_tests.rs",
        "proof/filelimit.rs",
        "proof/alarm_tests.rs",
        "proof/alarm_faults.rs",
        "proof/tests.rs",
    ];
    fn scan(dir: &std::path::Path, hits: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                scan(&p, hits);
            } else if p.extension().is_some_and(|x| x == "rs")
                && !ALLOWED.iter().any(|a| p.ends_with(a))
            {
                let text = std::fs::read_to_string(&p).unwrap();
                if NAMES.iter().any(|n| text.contains(n)) {
                    hits.push(p.display().to_string());
                }
            }
        }
    }
    let mut hits = Vec::new();
    scan(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut hits,
    );
    assert!(hits.is_empty(), "named outside the proof path: {hits:?}");
}
