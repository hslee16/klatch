//! Tests of the shared gateway (#140): one owner thread, many connection threads, a temp store on disk.

use super::shared::SharedGateway;
use crate::domain::{Error, Principal};
use crate::gateway::{
    AllowAll, ClaimSpec, Gateway, HistoryRange, SystemClock, Transport, pause_for_change,
};
use crate::store::SqliteStore;
use crate::store::test_dir::temp_db;
use std::collections::HashSet;
use std::time::{Duration, Instant};

const ROOM: &str = "r";

/// A shared gateway over a fresh SQLCipher file in a temp directory (never the live store).
fn shared(tag: &str) -> (SharedGateway, crate::store::test_dir::TempDb) {
    let db = temp_db(tag);
    let path = db.to_path_buf();
    let gw = SharedGateway::spawn(move || {
        let store = SqliteStore::open(&path, &crate::store::insecure_test_key())?;
        Ok(Gateway::new(store, SystemClock, AllowAll))
    })
    .unwrap();
    (gw, db)
}

fn as_member(gw: &SharedGateway, name: &str) -> impl Transport + Send + 'static {
    gw.transport(Principal::verified(name))
}

#[test]
fn threads_posting_through_one_handle_get_dense_unique_seqs_and_lose_nothing() {
    let (gw, _db) = shared("shared-post");
    let reader = as_member(&gw, "reader");
    reader.join(ROOM).unwrap();
    let workers: Vec<_> = (0..8)
        .map(|w| {
            let t = as_member(&gw, &format!("w{w}"));
            std::thread::spawn(move || {
                t.join(ROOM).unwrap();
                (0..25)
                    .map(|i| t.post(ROOM, &format!("w{w}-m{i}")).unwrap().seq)
                    .collect::<Vec<i64>>()
            })
        })
        .collect();
    let mut seqs: Vec<i64> = workers
        .into_iter()
        .flat_map(|h| h.join().unwrap())
        .collect();
    seqs.sort();
    assert_eq!(seqs, (1..=200).collect::<Vec<i64>>(), "dense, no repeat");

    let range = HistoryRange {
        since: None,
        last: Some(200),
    };
    let all = reader.history(ROOM, range).unwrap().messages;
    assert_eq!(all.len(), 200);
    assert!(all.windows(2).all(|p| p[0].seq + 1 == p[1].seq));
    let bodies: HashSet<_> = all.iter().map(|m| m.body.clone()).collect();
    assert_eq!(bodies.len(), 200, "no message lost or duplicated");
    for m in &all {
        let sender = m.body.split('-').next().unwrap();
        assert_eq!(m.sender, sender, "each post is attributed to its own handle");
    }
}

#[test]
fn readers_running_beside_writers_see_every_message_once_in_order() {
    let (gw, _db) = shared("shared-read");
    let reader = as_member(&gw, "reader");
    reader.join(ROOM).unwrap();
    let writers: Vec<_> = (0..4)
        .map(|w| {
            let t = as_member(&gw, &format!("w{w}"));
            std::thread::spawn(move || {
                t.join(ROOM).unwrap();
                for i in 0..25 {
                    t.post(ROOM, &format!("w{w}-m{i}")).unwrap();
                }
            })
        })
        .collect();
    let mut seen = Vec::new();
    let end = Instant::now() + Duration::from_secs(20);
    while seen.len() < 100 && Instant::now() < end {
        seen.extend(reader.read(ROOM, true).unwrap());
    }
    for w in writers {
        w.join().unwrap();
    }
    seen.extend(reader.read(ROOM, true).unwrap());
    let seqs: Vec<i64> = seen.iter().map(|m| m.seq).collect();
    assert_eq!(seqs, (1..=100).collect::<Vec<i64>>());
}

#[test]
fn eight_threads_claiming_one_path_give_exactly_one_holder() {
    let (gw, _db) = shared("shared-claim");
    let threads: Vec<_> = (0..8)
        .map(|w| {
            let t = as_member(&gw, &format!("w{w}"));
            std::thread::spawn(move || {
                t.join(ROOM).unwrap();
                (w, t.claim(ROOM, &ClaimSpec::new("src/lib.rs", 600, "edit")))
            })
        })
        .collect();
    let results: Vec<_> = threads.into_iter().map(|h| h.join().unwrap()).collect();
    let winners: Vec<_> = results.iter().filter(|(_, r)| r.is_ok()).collect();
    assert_eq!(winners.len(), 1, "exactly one holder");
    for (_, r) in results.iter().filter(|(_, r)| r.is_err()) {
        assert!(matches!(r, Err(Error::Conflict { .. })), "{r:?}");
    }
    let claims = as_member(&gw, "w0").claims(ROOM).unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].owner, format!("w{}", winners[0].0));
}

#[test]
fn a_bounded_wait_returns_when_another_thread_posts_and_blocks_nobody() {
    let (gw, _db) = shared("shared-wait");
    let waiter = as_member(&gw, "waiter");
    waiter.join(ROOM).unwrap();
    let poster = as_member(&gw, "poster");
    poster.join(ROOM).unwrap();
    let started = Instant::now();
    let waiting = std::thread::spawn(move || {
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            let seen = waiter.change_token();
            let got = waiter.wait_poll(ROOM).unwrap();
            if !got.is_empty() {
                return Some(got);
            }
            if Instant::now() >= end {
                return None;
            }
            pause_for_change(
                &|| waiter.change_token(),
                seen,
                Duration::from_secs(2),
                Duration::from_millis(20),
            );
        }
    });
    // While the waiter sits in its pause, other requests are served at once: the owner is never held by a wait.
    std::thread::sleep(Duration::from_millis(150));
    let before = Instant::now();
    assert!(poster.who(ROOM).is_ok());
    assert!(before.elapsed() < Duration::from_secs(1));
    poster.post(ROOM, "wake up").unwrap();
    let got = waiting.join().unwrap().expect("the wait saw the post");
    assert_eq!(got[0].body, "wake up");
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "woke on the post, not at the limit: {:?}",
        started.elapsed()
    );
}

#[test]
fn the_owner_stops_when_the_last_handle_is_dropped() {
    let (gw, db) = shared("shared-drop");
    let t = as_member(&gw, "a");
    t.join(ROOM).unwrap();
    drop(t);
    drop(gw);
    // The connection closed with the owner, so a new owner can open the same file and see the data.
    let again = SqliteStore::open(&db, &crate::store::insecure_test_key()).unwrap();
    let gw2 = Gateway::new(again, SystemClock, AllowAll);
    assert_eq!(gw2.rooms(&Principal::verified("a")).unwrap(), vec![ROOM]);
}

#[test]
fn a_gateway_that_cannot_be_built_is_an_error_not_a_hang() {
    let r = SharedGateway::spawn::<SqliteStore>(|| Err(Error::Backend("no store".into())));
    assert!(matches!(r, Err(Error::Backend(m)) if m == "no store"));
}

#[test]
fn a_request_that_panics_fails_that_call_and_leaves_the_owner_serving() {
    let (gw, _db) = shared("shared-panic");
    let t = as_member(&gw, "a");
    t.join(ROOM).unwrap();
    let r = gw.run_for_tests(|_| panic!("boom"));
    assert!(matches!(r, Err(Error::Backend(_))), "{r:?}");
    assert!(t.post(ROOM, "still here").is_ok());
}
