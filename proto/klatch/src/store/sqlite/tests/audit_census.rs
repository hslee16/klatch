//! The census of the audit (#1310) reads a bounded number of rows however large the audit is: it runs inside the write
//! transaction of every failed proof, so a cost that grew with the audit would be paid with the write lock held.

use super::*;
use crate::store::Census;
use crate::store::NewAudit;

const ACTION: &str = "census-demo";
const NOW: i64 = 1_000_000;
const WINDOW: i64 = 86_400;

fn q<'a>(action: &'a str, targets: &'a [&'a str], since: i64, cap: u64) -> Census<'a> {
    Census {
        action,
        targets,
        since,
        until: NOW + 1_000_000,
        cap,
    }
}

fn fill(s: &SqliteStore, old: usize, inside: usize) {
    s.conn.execute_batch("BEGIN").unwrap();
    for i in 0..old {
        let at = NOW - 2 * WINDOW - i as i64;
        s.record_audit(
            &NewAudit::new(ACTION, "", &format!("failures=1 terminal=old{i}")),
            at,
        )
        .unwrap();
    }
    for i in 0..inside {
        let target = if i % 5 == 0 { "recovery" } else { "" };
        let at = NOW - i as i64;
        let detail = format!("failures=1 terminal=t{}", i % 7);
        s.record_audit(&NewAudit::new(ACTION, target, &detail), at)
            .unwrap();
    }
    // noise the census must not count: another action, and another target of the same action
    s.record_audit(&NewAudit::new("other", "", "terminal=x"), NOW)
        .unwrap();
    s.record_audit(&NewAudit::new(ACTION, "elsewhere", "terminal=y"), NOW)
        .unwrap();
    s.conn.execute_batch("COMMIT").unwrap();
}

#[test]
fn old_rows_outside_the_window_are_not_counted_and_the_labels_are_distinct() {
    let s = SqliteStore::open_in_memory().unwrap();
    fill(&s, 5000, 49);
    let since = NOW - WINDOW + 1;
    let both = ["", "recovery"];
    assert_eq!(
        s.audit_census(&q(ACTION, &both, since, u64::MAX)).unwrap(),
        (49, 7)
    );
    // capped at 51 it is the same, since only 49 are inside; capped at 10 it stops at 10
    assert_eq!(
        s.audit_census(&q(ACTION, &both, since, 51)).unwrap(),
        (49, 7)
    );
    assert_eq!(s.audit_census(&q(ACTION, &both, since, 10)).unwrap().0, 10);
    assert_eq!(
        s.audit_census(&q(ACTION, &[""], since, u64::MAX))
            .unwrap()
            .0,
        39
    );
    assert_eq!(
        s.audit_census(&q(ACTION, &[], since, u64::MAX)).unwrap(),
        (0, 0)
    );
    assert_eq!(
        s.audit_census(&q("nothing", &both, since, u64::MAX))
            .unwrap(),
        (0, 0)
    );
}

#[test]
fn the_cap_stops_the_count_and_the_query_searches_the_index_by_range() {
    let s = SqliteStore::open_in_memory().unwrap();
    fill(&s, 5000, 80);
    let since = NOW - WINDOW + 1;
    let both = ["", "recovery"];
    assert_eq!(s.audit_census(&q(ACTION, &both, since, 51)).unwrap().0, 51);
    // the plan: every table access is a SEARCH of the (action, target, ts) index with the range on ts, none a SCAN
    let sql = format!(
        "EXPLAIN QUERY PLAN {}",
        crate::store::sqlite::api::policy::census_sql(2)
    );
    let mut st = s.conn.prepare(&sql).unwrap();
    let plan: Vec<String> = st
        .query_map(
            rusqlite::params_from_iter(crate::store::sqlite::api::policy::census_params(&q(
                ACTION, &both, since, 51,
            ))),
            |r| r.get::<_, String>(3),
        )
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    let searches: Vec<&String> = plan.iter().filter(|l| l.contains("admin_audit")).collect();
    assert_eq!(searches.len(), 2, "{plan:?}");
    for l in searches {
        assert!(
            l.starts_with("SEARCH")
                && l.contains("admin_audit_action_target_ts")
                && l.contains("action=?")
                && l.contains("target=?")
                && l.contains("ts>?"),
            "{plan:?}"
        );
    }
}
