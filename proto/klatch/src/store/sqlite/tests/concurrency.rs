use super::*;
use crate::domain::Error;

use std::time::Instant;

fn percentile(sorted: &[u128], p: f64) -> u128 {
    sorted[(((sorted.len() - 1) as f64) * p).round() as usize]
}

#[test]
fn concurrent_posters_get_dense_unique_seqs() {
    let path = temp_db("conc");
    SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap(); // create the schema once
    let handles: Vec<_> = (0..8)
        .map(|t| {
            let path = path.clone();
            std::thread::spawn(move || {
                let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
                (0..25)
                    .map(|i| {
                        s.post(Seat::new("r", &format!("w{t}")), "x", i)
                            .unwrap()
                            .seq
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let mut seqs: Vec<i64> = handles
        .into_iter()
        .flat_map(|h| h.join().unwrap())
        .collect();
    seqs.sort();
    assert_eq!(seqs, (1..=200).collect::<Vec<i64>>());
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn many_connections_opening_a_fresh_database_at_once_all_succeed() {
    for round in 0..30 {
        let path = temp_db(&format!("fresh{round}"));
        let gate = std::sync::Arc::new(std::sync::Barrier::new(12));
        let handles: Vec<_> = (0..12)
            .map(|_| {
                let (path, gate) = (path.clone(), gate.clone());
                std::thread::spawn(move || {
                    gate.wait();
                    SqliteStore::open(&path, &crate::store::insecure_test_key()).map(|_| ())
                })
            })
            .collect();
        for h in handles {
            h.join()
                .unwrap()
                .unwrap_or_else(|e| panic!("round {round}: {e}"));
        }
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}

#[test]
fn concurrent_claims_on_one_path_have_exactly_one_winner() {
    let path = temp_db("claimrace");
    SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    let handles: Vec<_> = (0..8)
        .map(|t| {
            let path = path.clone();
            std::thread::spawn(move || {
                let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
                s.claim(
                    &ClaimRequest::new(("r", &format!("w{t}")), "src", 100, ""),
                    0,
                )
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(
        results
            .iter()
            .filter(|r| r.is_err())
            .all(|r| matches!(r, Err(Error::Conflict { .. })))
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn concurrent_claims_on_disjoint_paths_all_succeed() {
    let path = temp_db("claimdisjoint");
    SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    let handles: Vec<_> = (0..8)
        .map(|t| {
            let path = path.clone();
            std::thread::spawn(move || {
                let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
                s.claim(
                    &ClaimRequest::new(("r", &format!("w{t}")), &format!("src/f{t}"), 100, ""),
                    0,
                )
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap().unwrap();
    }
    assert_eq!(
        SqliteStore::open(&path, &crate::store::insecure_test_key())
            .unwrap()
            .claims("r", 1)
            .unwrap()
            .len(),
        8
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn readers_and_writers_together_lose_and_duplicate_nothing() {
    let path = temp_db("rw");
    SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    let writers: Vec<_> = (0..4)
        .map(|t| {
            let path = path.clone();
            std::thread::spawn(move || {
                let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
                for i in 0..25 {
                    s.post(Seat::new("r", &format!("w{t}")), "x", i).unwrap();
                }
            })
        })
        .collect();
    let readers: Vec<_> = (0..3)
        .map(|t| {
            let path = path.clone();
            std::thread::spawn(move || {
                let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
                let me = format!("reader{t}");
                let mut seen = Vec::new();
                let deadline = Instant::now() + Duration::from_secs(30);
                while seen.len() < 100 && Instant::now() < deadline {
                    seen.extend(
                        s.read(Seat::new("r", &me), true, 0)
                            .unwrap()
                            .into_iter()
                            .map(|m| m.seq),
                    );
                }
                seen
            })
        })
        .collect();
    for w in writers {
        w.join().unwrap();
    }
    for r in readers {
        let seen = r.join().unwrap();
        assert_eq!(
            seen,
            (1..=100).collect::<Vec<i64>>(),
            "each reader sees every message once, in order"
        );
    }
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_held_write_lock_fails_a_write_after_the_timeout_and_never_hangs() {
    let path = temp_db("locked");
    SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    let s = SqliteStore::open_with_timeout(
        &path,
        &crate::store::insecure_test_key(),
        Duration::from_millis(100),
    )
    .unwrap();
    let holder = keyed(&path);
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = Instant::now();
    let err = s.post(Seat::new("r", "bob"), "x", 1).unwrap_err();
    assert!(matches!(err, Error::Busy(_)), "{err:?}");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "waited {:?}",
        started.elapsed()
    );
    holder.execute_batch("COMMIT").unwrap();
    assert_eq!(
        s.post(Seat::new("r", "bob"), "x", 2).unwrap().seq,
        1,
        "a failed post leaves no gap in seq"
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn pure_reads_proceed_while_another_connection_holds_the_write_lock() {
    let path = temp_db("readsok");
    let s = SqliteStore::open_with_timeout(
        &path,
        &crate::store::insecure_test_key(),
        Duration::from_millis(100),
    )
    .unwrap();
    s.claim(&ClaimRequest::new(("r", "bob"), "src", 100, ""), 0)
        .unwrap();
    let holder = keyed(&path);
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert_eq!(s.claims("r", 1).unwrap().len(), 1);
    assert_eq!(s.members("r").unwrap().len(), 0);
    holder.execute_batch("COMMIT").unwrap();
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// Run by hand: `cargo test --release -p klatch -- --ignored --nocapture`. Prints, not asserts.
#[test]
#[ignore]
fn measure_hook_path_latency_under_write_load() {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    for pause_us in [0u64, 1_000, 10_000, 100_000] {
        let path = temp_db(&format!("bench{pause_us}"));
        SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let posted = std::sync::Arc::new(AtomicU64::new(0));
        let writers: Vec<_> = (0..4)
            .map(|t| {
                let (path, stop, posted) = (path.clone(), stop.clone(), posted.clone());
                std::thread::spawn(move || {
                    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
                    while !stop.load(Ordering::Relaxed) {
                        s.post(Seat::new("r", &format!("w{t}")), "x", 0).unwrap();
                        posted.fetch_add(1, Ordering::Relaxed);
                        std::thread::sleep(Duration::from_micros(pause_us));
                    }
                })
            })
            .collect();
        let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        let (mut read_us, mut claims_us) = (Vec::new(), Vec::new());
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            let t0 = Instant::now();
            s.read(Seat::new("r", "reader"), true, 0).unwrap();
            read_us.push(t0.elapsed().as_micros());
            let t1 = Instant::now();
            s.claims("r", 0).unwrap();
            claims_us.push(t1.elapsed().as_micros());
            std::thread::sleep(Duration::from_millis(10));
        }
        stop.store(true, Ordering::Relaxed);
        for w in writers {
            w.join().unwrap();
        }
        read_us.sort();
        claims_us.sort();
        println!(
            "writers pause {pause_us}us: {} posts/s | read p50={}us p99={}us max={}us | claims p50={}us p99={}us (n={})",
            posted.load(Ordering::Relaxed) / 5,
            percentile(&read_us, 0.5),
            percentile(&read_us, 0.99),
            read_us.last().unwrap(),
            percentile(&claims_us, 0.5),
            percentile(&claims_us, 0.99),
            read_us.len()
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}

/// A store whose writer lock another connection can hold, so any write fails with `Busy` after 100 ms. Returns the
/// store, a second connection that can hold the lock, and the database path.
fn store_with_lock_holder(name: &str) -> (SqliteStore, rusqlite::Connection, std::path::PathBuf) {
    let path = temp_db(name);
    let s = SqliteStore::open_with_timeout(
        &path,
        &crate::store::insecure_test_key(),
        Duration::from_millis(100),
    )
    .unwrap();
    let holder = keyed(&path);
    (s, holder, path)
}

#[test]
fn a_hook_with_nothing_to_show_in_the_same_second_does_not_take_the_writer_lock() {
    let (s, holder, path) = store_with_lock_holder("hookidle");
    s.join("r", "alice", 10).unwrap();
    s.post(Seat::new("r", "bob"), "old", 10).unwrap();
    s.hook_read(Seat::new("r", "alice"), 10).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let shown = s.hook_read(Seat::new("r", "alice"), 10);
    holder.execute_batch("COMMIT").unwrap();
    assert!(
        shown
            .expect("a read-only check needs no writer lock")
            .is_empty(),
        "nothing new to show"
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_hook_with_something_to_show_still_records_it_under_the_writer_lock() {
    let (s, holder, path) = store_with_lock_holder("hookbusy");
    s.join("r", "alice", 10).unwrap();
    s.hook_read(Seat::new("r", "alice"), 10).unwrap();
    s.post(Seat::new("r", "bob"), "new", 10).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let shown = s.hook_read(Seat::new("r", "alice"), 10);
    holder.execute_batch("COMMIT").unwrap();
    assert!(
        matches!(shown, Err(Error::Busy(_))),
        "a message to mark shown needs the writer lock: {shown:?}"
    );
    assert_eq!(s.hook_read(Seat::new("r", "alice"), 10).unwrap().len(), 1);
    assert!(s.hook_read(Seat::new("r", "alice"), 10).unwrap().is_empty());
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_later_second_still_refreshes_last_seen() {
    let (s, holder, path) = store_with_lock_holder("hookseen");
    drop(holder);
    s.join("r", "alice", 10).unwrap();
    s.hook_read(Seat::new("r", "alice"), 10).unwrap();
    s.hook_read(Seat::new("r", "alice"), 11).unwrap();
    let seen = |s: &SqliteStore| s.members("r").unwrap()[0].last_seen;
    assert_eq!(seen(&s), 11, "a hook in a later second moves last_seen");
    s.wait_poll(Seat::new("r", "alice"), 12).unwrap();
    assert_eq!(seen(&s), 12, "so does a wait poll");
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_wait_poll_in_the_same_second_does_not_take_the_writer_lock() {
    let (s, holder, path) = store_with_lock_holder("waitidle");
    s.join("r", "alice", 10).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let polled = s.wait_poll(Seat::new("r", "alice"), 10);
    holder.execute_batch("COMMIT").unwrap();
    assert!(
        polled
            .expect("nothing changed, nothing to write")
            .is_empty(),
        "no messages"
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn claim_changes_with_nothing_new_does_not_take_the_writer_lock() {
    let (s, holder, path) = store_with_lock_holder("changesidle");
    s.claim(&ClaimRequest::new(("r", "bob"), "src", 100, ""), 0)
        .unwrap();
    s.claim_changes("r", "alice", 1).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let changes = s.claim_changes("r", "alice", 2);
    holder.execute_batch("COMMIT").unwrap();
    let changes = changes.expect("an unchanged claim list needs no writer lock");
    assert!(changes.added.is_empty() && changes.released.is_empty());
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn claim_changes_with_a_change_still_records_it_under_the_writer_lock() {
    let (s, holder, path) = store_with_lock_holder("changesbusy");
    s.claim_changes("r", "alice", 1).unwrap();
    s.claim(&ClaimRequest::new(("r", "bob"), "src", 100, ""), 1)
        .unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let changes = s.claim_changes("r", "alice", 2);
    holder.execute_batch("COMMIT").unwrap();
    assert!(matches!(changes, Err(Error::Busy(_))), "{changes:?}");
    assert_eq!(s.claim_changes("r", "alice", 2).unwrap().added.len(), 1);
    assert!(s.claim_changes("r", "alice", 3).unwrap().added.is_empty());
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// Run by hand like the test above: the hook path (`hook_read` then `claim_changes`, as the digest calls them) and a
/// wait poll under four writers, with the clock in real seconds. Prints, not asserts.
#[test]
#[ignore]
fn measure_hook_digest_path_under_write_load() {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    let secs = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    };
    for pause_us in [0u64, 1_000, 10_000, 100_000] {
        let path = temp_db(&format!("digest{pause_us}"));
        SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let posted = std::sync::Arc::new(AtomicU64::new(0));
        let writers: Vec<_> = (0..4)
            .map(|t| {
                let (path, stop, posted) = (path.clone(), stop.clone(), posted.clone());
                std::thread::spawn(move || {
                    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
                    while !stop.load(Ordering::Relaxed) {
                        // a post that times out under saturation is not a post
                        if s.post(Seat::new("r", &format!("w{t}")), "x", secs())
                            .is_ok()
                        {
                            posted.fetch_add(1, Ordering::Relaxed);
                        }
                        std::thread::sleep(Duration::from_micros(pause_us));
                    }
                })
            })
            .collect();
        let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        let mut timed_out = 0;
        s.claim(&ClaimRequest::new(("r", "w0"), "src", 100_000, ""), secs())
            .unwrap();
        let (mut hook_us, mut wait_us) = (Vec::new(), Vec::new());
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            let t0 = Instant::now();
            let hook = s.hook_read(Seat::new("r", "reader"), secs()).map(|_| ());
            let hook = hook.and(s.claim_changes("r", "reader", secs()).map(|_| ()));
            hook_us.push(t0.elapsed().as_micros());
            let t1 = Instant::now();
            let wait = s.wait_poll(Seat::new("r", "waiter"), secs());
            wait_us.push(t1.elapsed().as_micros());
            timed_out += usize::from(hook.is_err()) + usize::from(wait.is_err());
            std::thread::sleep(Duration::from_millis(10));
        }
        stop.store(true, Ordering::Relaxed);
        for w in writers {
            w.join().unwrap();
        }
        hook_us.sort();
        wait_us.sort();
        println!(
            "writers pause {pause_us}us: {} posts/s | hook p50={}us p99={}us max={}us | wait_poll p50={}us p99={}us max={}us (n={}, timed out {timed_out})",
            posted.load(Ordering::Relaxed) / 5,
            percentile(&hook_us, 0.5),
            percentile(&hook_us, 0.99),
            hook_us.last().unwrap(),
            percentile(&wait_us, 0.5),
            percentile(&wait_us, 0.99),
            wait_us.last().unwrap(),
            hook_us.len()
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}
