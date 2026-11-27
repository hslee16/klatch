//! The order of a listing and the scan bound, pinned (#1706).

use super::*;
use std::time::{Duration, SystemTime};

fn touch(dir: &Path, pid: u32, when: SystemTime) {
    let f = dir.join(format!("{pid}.json"));
    std::fs::write(&f, "{}").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&f)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

#[test]
fn a_time_ahead_of_now_counts_as_now_so_only_the_name_orders_the_files_ahead() {
    // without the clamp to now a file 23 h ahead would outrank one 1 h ahead whatever its name
    let d = crate::store::test_dir::TestDir::new("rank-clamp");
    let now = SystemTime::now();
    touch(&d, 100, now + Duration::from_secs(3600));
    touch(&d, 200, now + Duration::from_secs(23 * 3600));
    touch(&d, 300, now - Duration::from_secs(60));
    let names = ordered_record_names(&d, 100, 0, None).unwrap().0;
    assert_eq!(names, ["100.json", "200.json", "300.json"]);
}

#[test]
fn running_pids_come_before_newer_files_of_pids_that_do_not_run() {
    let d = crate::store::test_dir::TestDir::new("rank-running");
    let now = SystemTime::now();
    touch(&d, 100, now);
    touch(&d, 200, now - Duration::from_secs(30 * 86_400));
    let alive: HashSet<u32> = [200].into_iter().collect();
    let names = ordered_record_names(&d, 100, 0, Some(&alive)).unwrap().0;
    assert_eq!(names, ["200.json", "100.json"]);
}

#[test]
fn the_scan_bound_is_65536_names_and_covers_what_is_opened() {
    assert_eq!(MAX_SCANNED, 65_536);
    const _: () = assert!(MAX_SCANNED >= MAX_OPENED && MAX_OPENED >= MAX_ENTRIES);
}

fn valid(dir: &Path, pid: u32) {
    let json = format!(
        r#"{{"pid":{pid},"sessionId":"s{pid}","cwd":"/w","procStart":"x","pidDomain":"darwin"}}"#
    );
    std::fs::write(dir.join(format!("{pid}.json")), json).unwrap();
}

fn scan(dir: &Path, scanned: usize, records: usize, opened: usize) -> Scan {
    scan_records_bounded(dir, scanned, records, opened, None, |n| {
        read_regular_named(dir, n)
    })
    .unwrap()
}

#[test]
fn a_listing_cut_at_the_record_bound_says_so_and_one_that_fits_does_not() {
    // #1706: a caller acting on a cut listing acts on a view that may lack the record that matters
    let d = crate::store::test_dir::TestDir::new("trunc-records");
    for pid in 1..=4 {
        valid(&d, pid);
    }
    let fits = scan(&d, 100, 4, 100);
    assert_eq!(fits.records.len(), 4);
    assert!(!fits.truncated, "exactly the bound is not a cut");
    let cut = scan(&d, 100, 3, 100);
    assert_eq!(cut.records.len(), 3);
    assert!(cut.truncated);
}

#[test]
fn a_listing_cut_by_the_open_or_scan_bound_says_so() {
    let d = crate::store::test_dir::TestDir::new("trunc-open");
    for pid in 1..=4 {
        valid(&d, pid);
    }
    assert!(scan(&d, 100, 100, 3).truncated, "open bound");
    assert!(scan(&d, 3, 100, 100).truncated, "scan bound");
    assert!(!scan(&d, 4, 100, 4).truncated);
}

#[test]
fn a_cut_that_leaves_only_pids_that_do_not_run_is_not_a_truncation_that_matters() {
    // the running pids are read first, so what a bound leaves unread can only read as gone
    let d = crate::store::test_dir::TestDir::new("trunc-running");
    for pid in 1..=4 {
        valid(&d, pid);
    }
    let read = |n: &str| read_regular_named(&d, n);
    let only_dead: HashSet<u32> = [1, 2].into_iter().collect();
    let s = scan_records_bounded(&d, 100, 3, 100, Some(&only_dead), read).unwrap();
    assert_eq!(s.records.len(), 3);
    assert!(!s.truncated, "one dead pid unread");
    let all: HashSet<u32> = (1..=4).collect();
    let s = scan_records_bounded(&d, 100, 3, 100, Some(&all), read).unwrap();
    assert!(s.truncated, "a running pid unread");
}
