//! The change token (#616, #148): a number that differs whenever another connection committed, so a waiter can sleep
//! on it instead of polling the tables.

use super::*;
use crate::store::MessageStore;

fn token(s: &SqliteStore) -> Option<u64> {
    s.change_token()
}

#[test]
fn the_token_is_the_same_while_nothing_else_commits() {
    let path = temp_db("tokenquiet");
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    let first = token(&s);
    assert!(first.is_some(), "a sqlite store has a token");
    assert_eq!(first, token(&s));
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn the_token_differs_after_another_connection_commits() {
    let path = temp_db("tokenother");
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    let other = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    let before = token(&s);
    other.post(Seat::new("r", "bob"), "hi", 1).unwrap();
    assert_ne!(before, token(&s), "a post by another connection shows");
    let after = token(&s);
    assert_eq!(after, token(&s), "and the token settles again");
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_commit_between_the_token_and_the_poll_is_never_missed() {
    // The waiter takes the token, then polls. A message posted after the token was taken must either show in the poll
    // or make the next token differ.
    let path = temp_db("tokenrace");
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    let other = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    s.join("r", "alice", 1).unwrap();
    let before = token(&s);
    other.post(Seat::new("r", "bob"), "hi", 2).unwrap();
    let polled = s.wait_poll(Seat::new("r", "alice"), 2).unwrap();
    assert!(polled.len() == 1 || token(&s) != before);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

fn secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// One waiter as `klatch wait` runs it: take the token, poll, then pause (on the token or on the old fixed 200 ms
/// sleep) until the poll finds a message or `stop` is set. Counts its polls; returns when it woke, if it did.
fn waiter(
    (path, me): (&std::path::Path, &str),
    on_token: bool,
    (polls, stop): (
        &std::sync::atomic::AtomicU64,
        &std::sync::atomic::AtomicBool,
    ),
) -> Option<std::time::Instant> {
    use std::sync::atomic::Ordering::Relaxed;
    let s = SqliteStore::open(path, &crate::store::insecure_test_key()).unwrap();
    while !stop.load(Relaxed) {
        let seen = s.change_token();
        polls.fetch_add(1, Relaxed);
        if !s.wait_poll(Seat::new("r", me), secs()).unwrap().is_empty() {
            return Some(std::time::Instant::now());
        }
        if on_token {
            let (limit, tick) = (crate::gateway::WAKE_FALLBACK, crate::gateway::WAKE_TICK);
            crate::gateway::pause_for_change(&|| s.change_token(), seen, limit, tick);
        } else {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }
    None
}

/// Run by hand: `cargo test --release -p klatch --lib -- --ignored --nocapture measure_idle_waiters`. Twenty waiters
/// on their own connections wait 4 s on an idle room, once with the old fixed 200 ms sleep and once waking on the
/// token; prints the polls each made, and how long the slowest took to see a post.
#[test]
#[ignore]
fn measure_idle_waiters_and_wake_latency() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
    use std::time::{Duration, Instant};
    for on_token in [false, true] {
        let path = temp_db(if on_token { "idletoken" } else { "idletimer" });
        SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        let (polls, stop) = (
            Arc::new(AtomicU64::new(0)),
            Arc::new(AtomicBool::new(false)),
        );
        let waiters: Vec<_> = (0..20)
            .map(|i| {
                let (path, polls, stop) = (path.to_path_buf(), polls.clone(), stop.clone());
                std::thread::spawn(move || {
                    waiter((&path, &format!("w{i}")), on_token, (&polls, &stop))
                })
            })
            .collect();
        std::thread::sleep(Duration::from_secs(4));
        let idle_polls = polls.load(Relaxed);
        let poster = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        let posted = Instant::now();
        poster
            .post(Seat::new("r", "poster"), "wake", secs())
            .unwrap();
        let worst = waiters
            .into_iter()
            .filter_map(|w| w.join().unwrap())
            .map(|t| t.duration_since(posted).as_millis())
            .max()
            .unwrap();
        let name = if on_token {
            "token wake"
        } else {
            "200 ms timer"
        };
        println!(
            "{name}: 20 idle waiters made {idle_polls} polls in 4 s ({:.1} per waiter per second); worst wake after a post {worst} ms",
            idle_polls as f64 / 20.0 / 4.0
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}

#[test]
fn an_encrypted_store_has_a_token_too() {
    let path = temp_db("tokencipher");
    let key = crate::store::insecure_test_key();
    let s = SqliteStore::open(&path, &key).unwrap();
    let other = SqliteStore::open(&path, &key).unwrap();
    let before = token(&s);
    other.post(Seat::new("r", "bob"), "hi", 1).unwrap();
    assert_ne!(before, token(&s));
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}
