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
    let names = ordered_record_names(&d, 100, 0, None).unwrap();
    assert_eq!(names, ["100.json", "200.json", "300.json"]);
}

#[test]
fn running_pids_come_before_newer_files_of_pids_that_do_not_run() {
    let d = crate::store::test_dir::TestDir::new("rank-running");
    let now = SystemTime::now();
    touch(&d, 100, now);
    touch(&d, 200, now - Duration::from_secs(30 * 86_400));
    let alive: HashSet<u32> = [200].into_iter().collect();
    let names = ordered_record_names(&d, 100, 0, Some(&alive)).unwrap();
    assert_eq!(names, ["200.json", "100.json"]);
}

#[test]
fn the_scan_bound_is_65536_names_and_covers_what_is_opened() {
    assert_eq!(MAX_SCANNED, 65_536);
    const _: () = assert!(MAX_SCANNED >= MAX_OPENED && MAX_OPENED >= MAX_ENTRIES);
}
