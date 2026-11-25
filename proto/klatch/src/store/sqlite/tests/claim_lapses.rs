//! The record of lapsed claims (#1586): a named claim that lapses is written to `claim_lapses` by the sweep, so the
//! `lapsed_claim` stuck event survives the next claim attempt's sweep. Bounded by the window and a cap per room;
//! passive, ask and released claims are never recorded; it holds room, path, owner and times, never a reason.

use super::*;
use crate::domain::{LAPSE_RECORD_CAP, LAPSED_CLAIM_MAX, LAPSED_WINDOW_SECS, PASSIVE_REASON};

const WINDOW: (i64, usize) = (LAPSED_WINDOW_SECS, LAPSED_CLAIM_MAX);

fn take(s: &SqliteStore, (room, who, path): (&str, &str, &str), ttl: i64, now: i64) {
    s.claim(&ClaimRequest::new((room, who), path, ttl, ""), now)
        .unwrap();
}

fn lapses(s: &SqliteStore, room: &str, now: i64) -> Vec<(String, String, i64)> {
    s.lapsed_claims(room, now, WINDOW).unwrap()
}

fn rows(s: &SqliteStore) -> i64 {
    s.conn
        .query_row("SELECT COUNT(*) FROM claim_lapses", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn a_lapse_survives_a_sweep_triggered_by_a_claim_attempt_in_another_room() {
    let s = store();
    take(&s, ("r", "bob", "gh-watch:receiver"), 100, 0);
    // an attempt in another room sweeps every room's lapsed rows
    take(&s, ("other", "alice", "x"), 50_000, 150);
    assert!(s.claims("r", 150).unwrap().is_empty(), "swept");
    assert_eq!(
        lapses(&s, "r", 160),
        [("gh-watch:receiver".to_string(), "bob".to_string(), 100)]
    );
    assert!(lapses(&s, "other", 160).is_empty(), "room isolation");
}

#[test]
fn a_lapse_is_reported_inside_the_window_and_not_after_it() {
    let s = store();
    take(&s, ("r", "bob", "job"), 100, 0);
    take(&s, ("r", "alice", "x"), 50_000, 150);
    assert_eq!(lapses(&s, "r", 100 + LAPSED_WINDOW_SECS - 1).len(), 1);
    assert!(lapses(&s, "r", 100 + LAPSED_WINDOW_SECS).is_empty());
}

#[test]
fn a_record_past_the_window_is_pruned_by_the_next_sweep() {
    let s = store();
    take(&s, ("r", "bob", "job"), 100, 0);
    take(&s, ("r", "alice", "x"), 50_000, 150);
    assert_eq!(rows(&s), 1);
    // a later sweep with something lapsing prunes the old record
    let later = 100 + LAPSED_WINDOW_SECS + 5;
    take(&s, ("r", "carol", "y"), 10, later);
    take(&s, ("r", "dave", "z"), 50_000, later + 20);
    assert_eq!(rows(&s), 1, "only the new lapse (y) remains");
    assert_eq!(lapses(&s, "r", later + 20)[0].0, "y");
}

#[test]
fn a_flood_of_lapses_is_capped_per_room_and_the_oldest_are_dropped() {
    let s = store();
    let n = LAPSE_RECORD_CAP + 30;
    for i in 0..n {
        // each lapses one second after the one before
        take(&s, ("r", "bob", &format!("job:{i}")), 1000, i as i64);
    }
    take(&s, ("r", "alice", "sweep"), 50_000, 1200);
    let kept: Vec<String> = s
        .rows(
            "SELECT path FROM claim_lapses WHERE room='r' AND owner='bob'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows(&s) as usize, LAPSE_RECORD_CAP, "the cap");
    assert_eq!(kept.len(), LAPSE_RECORD_CAP);
    assert!(kept.contains(&format!("job:{}", n - 1)), "newest kept");
    assert!(!kept.contains(&"job:0".to_string()), "oldest dropped");
}

#[test]
fn the_cap_is_per_room_so_a_flood_in_one_room_does_not_evict_another() {
    let s = store();
    take(&s, ("quiet", "bob", "gh-watch:receiver"), 1000, 0);
    for i in 0..(LAPSE_RECORD_CAP + 10) {
        take(&s, ("noisy", "eve", &format!("j{i}")), 1000, i as i64);
    }
    take(&s, ("noisy", "eve", "sweep"), 50_000, 1200);
    assert_eq!(lapses(&s, "quiet", 1201).len(), 1);
}

#[test]
fn passive_ask_and_released_claims_are_never_recorded() {
    let s = store();
    s.claim(
        &ClaimRequest::new(("r", "bob"), "src/a.rs", 100, PASSIVE_REASON),
        0,
    )
    .unwrap();
    take(&s, ("r", "bob", "ask:7"), 100, 0);
    take(&s, ("r", "bob", "job"), 100, 0);
    s.release_at("r", "bob", "job", 5).unwrap();
    take(&s, ("r", "alice", "x"), 50_000, 150);
    assert_eq!(rows(&s), 0);
    assert!(lapses(&s, "r", 151).is_empty());
}

#[test]
fn a_reclaim_by_the_same_holder_clears_the_lapse() {
    let s = store();
    take(&s, ("r", "bob", "gh-watch:receiver"), 100, 0);
    take(&s, ("r", "bob", "gh-watch:receiver"), 100, 150);
    assert!(lapses(&s, "r", 151).is_empty(), "the producer is back");
}

#[test]
fn a_repeated_lapse_of_one_claim_is_one_record() {
    let s = store();
    for i in 0..5 {
        take(&s, ("r", "bob", "job"), 10, i * 100);
    }
    take(&s, ("r", "alice", "x"), 50_000, 600);
    assert_eq!(rows(&s), 1);
}

#[test]
fn the_record_holds_no_reason() {
    let s = store();
    s.claim(
        &ClaimRequest::new(("r", "bob"), "job", 100, "reason-for-the-giraffe-claim"),
        0,
    )
    .unwrap();
    take(&s, ("r", "alice", "x"), 50_000, 150);
    let cols: String = s
        .conn
        .query_row(
            "SELECT group_concat(name) FROM pragma_table_info('claim_lapses')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cols, "room,path,owner,expires,since");
    let n: i64 = s
        .conn
        .query_row(
            "SELECT COUNT(*) FROM claim_lapses WHERE path LIKE '%giraffe%' OR owner LIKE '%giraffe%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn the_table_lives_in_the_encrypted_store_and_nothing_is_readable_outside() {
    let path = temp_db("lapses-enc");
    {
        let s = SqliteStore::open(&path, &crate::keys::Key::from_bytes([7; 32])).unwrap();
        take(
            &s,
            (
                "vault-room-name",
                "member-quokkaworth",
                "claim/path/zebra.rs",
            ),
            100,
            0,
        );
        take(&s, ("vault-room-name", "alice", "x"), 50_000, 150);
        assert_eq!(rows(&s), 1);
        let mut all = std::fs::read(&path).unwrap_or_default();
        for ext in ["-wal", "-journal", "-shm"] {
            let mut p = path.as_os_str().to_owned();
            p.push(ext);
            all.extend(std::fs::read(std::path::PathBuf::from(p)).unwrap_or_default());
        }
        for secret in [
            "member-quokkaworth",
            "claim/path/zebra.rs",
            "vault-room-name",
        ] {
            assert!(
                !all.windows(secret.len()).any(|w| w == secret.as_bytes()),
                "{secret}"
            );
        }
        let in_main: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM main.sqlite_master WHERE name='claim_lapses'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            in_main, 1,
            "in the one SQLCipher database, no attached file"
        );
    }
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_holder_that_posted_done_after_taking_the_claim_is_not_reported_even_after_the_sweep() {
    let s = store();
    s.join("r", "bob", 1).unwrap();
    take(&s, ("r", "bob", "job"), 100, 2);
    s.set_status(Seat::new("r", "bob"), "done: all", 50)
        .unwrap();
    take(&s, ("r", "alice", "x"), 50_000, 150);
    assert!(lapses(&s, "r", 151).is_empty());
}
