//! The record of lapsed claims (#1586): a named claim that lapses is written to `claim_lapses` by the sweep, so the
//! `lapsed_claim` stuck event survives the next claim attempt's sweep. Bounded by the window and a cap per room;
//! passive, ask and released claims are never recorded; it holds room, path, owner and times, never a reason.

use super::*;
use crate::domain::{
    LAPSE_RECORD_CAP, LAPSE_TOTAL_CAP, LAPSED_CLAIM_MAX, LAPSED_WINDOW_SECS, PASSIVE_REASON,
};

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

#[test]
fn a_reclaim_then_a_clean_release_leaves_no_lapse_to_report() {
    let s = store();
    take(&s, ("r", "bob", "watch:gh"), 100, 0);
    take(&s, ("r", "bob", "watch:gh"), 100, 150);
    s.release_at("r", "bob", "watch:gh", 160).unwrap();
    assert!(lapses(&s, "r", 161).is_empty());
    assert_eq!(rows(&s), 0, "the record went with the re-grant");
}

#[test]
fn a_passive_regrant_by_the_same_holder_also_clears_the_record() {
    let s = store();
    take(&s, ("r", "bob", "src/a.rs"), 100, 0);
    take(&s, ("r", "alice", "x"), 50_000, 150);
    assert_eq!(rows(&s), 1);
    s.claim(
        &ClaimRequest::new(("r", "bob"), "src/a.rs", 100, PASSIVE_REASON),
        160,
    )
    .unwrap();
    assert_eq!(rows(&s), 0);
}

#[test]
fn a_repeated_lapse_carries_the_new_expiry_and_since_and_a_done_between_does_not_hide_it() {
    let s = store();
    s.join("r", "bob", 1).unwrap();
    take(&s, ("r", "bob", "job"), 100, 2); // first lease: since 2, lapses at 102
    take(&s, ("r", "alice", "x"), 50_000, 150); // the sweep records it
    s.set_status(Seat::new("r", "bob"), "done: first", 160)
        .unwrap();
    // a second lease of the same holder and path that is swept while the first record still stands
    s.conn
        .execute(
            "INSERT INTO claims(room,path,owner,member,reason,expires,since) VALUES('r','job','bob','bob','',300,200)",
            [],
        )
        .unwrap();
    take(&s, ("r", "alice", "y"), 50_000, 400);
    let (expires, since): (i64, i64) = s
        .conn
        .query_row(
            "SELECT expires,since FROM claim_lapses WHERE path='job' AND owner='bob'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((expires, since), (300, 200), "the second lease's values");
    assert_eq!(
        lapses(&s, "r", 401).len(),
        1,
        "done (160) predates since 200"
    );
}

#[test]
fn a_swept_record_and_a_newer_unswept_row_for_one_holder_and_path_are_one_event() {
    let s = store();
    take(&s, ("r", "bob", "job"), 100, 0);
    take(&s, ("r", "alice", "x"), 50_000, 150); // sweeps bob's first lease (expires 100)
    // a newer lease of the same holder and path that nothing has swept
    s.conn
        .execute(
            "INSERT INTO claims(room,path,owner,member,reason,expires,since) VALUES('r','job','bob','bob','',200,160)",
            [],
        )
        .unwrap();
    // simulate the record of the older lapse still standing next to the newer unswept row
    s.conn
        .execute(
            "INSERT OR REPLACE INTO claim_lapses(room,path,owner,expires,since) VALUES('r','job','bob',100,0)",
            [],
        )
        .unwrap();
    assert_eq!(
        lapses(&s, "r", 250),
        [("job".to_string(), "bob".to_string(), 200)]
    );
}

#[test]
fn a_record_past_the_window_is_pruned_by_a_sweep_in_which_nothing_lapsed() {
    let s = store();
    take(&s, ("r", "bob", "job"), 100, 0);
    take(&s, ("r", "alice", "x"), 50_000, 150);
    assert_eq!(rows(&s), 1);
    // no lapse at all this time: an attempt long after the window still clears the old record
    take(
        &s,
        ("r", "carol", "y"),
        50_000,
        100 + LAPSED_WINDOW_SECS + 5,
    );
    assert_eq!(rows(&s), 0);
}

#[test]
fn the_total_across_rooms_is_capped_and_the_oldest_lapses_go_first() {
    let s = store();
    let n = LAPSE_TOTAL_CAP + 20;
    for i in 0..n {
        // one lapse per room, all taken at once and lapsing in order, four to a second
        take(
            &s,
            (&format!("room{i}"), "bob", "job"),
            10 + i as i64 / 4,
            0,
        );
    }
    take(&s, ("sweeper", "alice", "x"), 50_000, 400);
    assert_eq!(rows(&s) as usize, LAPSE_TOTAL_CAP, "the total cap");
    assert_eq!(
        lapses(&s, &format!("room{}", n - 1), 401).len(),
        1,
        "newest kept"
    );
    assert!(lapses(&s, "room0", 401).is_empty(), "oldest dropped");
}

#[test]
fn a_removed_member_has_no_lapse_left_to_report() {
    let s = store();
    s.join("r", "bob", 0).unwrap();
    take(&s, ("r", "bob", "job"), 100, 0);
    take(&s, ("r", "dave", "other"), 100, 0);
    take(&s, ("r", "alice", "x"), 50_000, 150);
    assert_eq!(lapses(&s, "r", 160).len(), 2);
    s.remove_member("r", "bob", 160).unwrap();
    let left = lapses(&s, "r", 161);
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0].1, "dave", "other holders keep their record");
}

#[test]
fn a_sweep_of_many_lapses_records_each_as_an_expiry() {
    let s = store();
    for i in 0..30 {
        take(&s, ("r", "bob", &format!("job{i}")), 100, 0);
    }
    take(&s, ("r", "alice", "x"), 50_000, 150);
    let expired: i64 = s
        .conn
        .query_row(
            "SELECT COUNT(*) FROM claim_events WHERE event='expired'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(expired, 30);
}

#[test]
fn a_flood_over_many_rooms_does_not_push_out_the_lapse_of_a_quiet_room() {
    // #1702: eleven rooms at the per-room cap, one more room with one older lapse; by age alone the quiet room's lapse
    // is the first to go, silently
    let s = store();
    take(&s, ("quiet", "bob", "gh-watch:receiver"), 50, 0);
    for room in 0..11 {
        for i in 0..LAPSE_RECORD_CAP {
            take(
                &s,
                (&format!("noisy{room}"), "eve", &format!("j{i}")),
                100,
                0,
            );
        }
    }
    take(&s, ("sweeper", "alice", "x"), 50_000, 150);
    assert_eq!(rows(&s) as usize, LAPSE_TOTAL_CAP);
    assert_eq!(
        lapses(&s, "quiet", 151).len(),
        1,
        "the quiet room kept its lapse"
    );
    assert_eq!(s.claim_lapses_dropped(), 101, "and the drop is counted");
}

#[test]
fn rows_dropped_by_the_per_room_cap_are_counted_too() {
    let s = store();
    for i in 0..(LAPSE_RECORD_CAP + 7) {
        take(&s, ("r", "eve", &format!("j{i}")), 100, 0);
    }
    take(&s, ("r", "alice", "x"), 50_000, 150);
    assert_eq!(s.claim_lapses_dropped(), 7);
}

fn yield_rows(s: &SqliteStore) -> Vec<(String, String)> {
    s.rows(
        "SELECT member,path FROM claim_yields WHERE path<>'' ORDER BY member,path",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

#[test]
fn a_sweep_reads_the_deadlock_marks_once_not_once_per_lapse() {
    // #1620: the sweep paid one indexed select per lapse in note_yield_end; the reads of a sweep must not grow with
    // the number of lapses
    let reads = |lapses: usize| {
        let s = store();
        for i in 0..lapses {
            take(&s, ("r", "bob", &format!("job{i}")), 100, 0);
        }
        let before = s.selects.get();
        take(&s, ("r", "alice", "x"), 50_000, 150);
        s.selects.get() - before
    };
    let (few, many) = (reads(2), reads(40));
    assert_eq!(few, many, "reads with 2 lapses {few}, with 40 {many}");
}

#[test]
fn a_lapse_of_a_deadlocked_member_is_marked_and_another_member_is_not() {
    // the behaviour the batched read must keep: only a holder that was in a deadlock inside the window before the
    // lease ended gets its path marked for a later take
    let s = store();
    take(&s, ("r", "bob", "a"), 100, 0);
    take(&s, ("r", "bob", "b"), 100, 0);
    take(&s, ("r", "dave", "c"), 100, 0);
    s.conn
        .execute(
            "INSERT INTO claim_yields(room,member,path,ts,taker) VALUES('r','bob','',50,'')",
            [],
        )
        .unwrap();
    take(&s, ("r", "alice", "x"), 50_000, 150);
    assert_eq!(
        yield_rows(&s),
        [("bob".into(), "a".into()), ("bob".into(), "b".into())]
    );
}

#[test]
fn a_deadlock_mark_older_than_the_window_before_the_lease_end_does_not_mark() {
    let s = store();
    take(&s, ("r", "bob", "a"), 100, 0);
    s.conn
        .execute(
            &format!(
                "INSERT INTO claim_yields(room,member,path,ts,taker) VALUES('r','bob','',{},'')",
                100 - crate::domain::YIELD_WINDOW_SECS - 1
            ),
            [],
        )
        .unwrap();
    take(&s, ("r", "alice", "x"), 50_000, 150);
    assert!(yield_rows(&s).is_empty());
}

#[test]
fn housekeeping_prunes_lapse_rows_past_the_window_on_a_store_with_no_claim_attempts() {
    // #1620: the prune ran only inside a claim sweep; an idle store kept its rows
    let s = store();
    take(&s, ("r", "bob", "job"), 100, 0);
    take(&s, ("r", "alice", "x"), 50_000, 150);
    assert_eq!(rows(&s), 1);
    s.prune_members(1_000_000, 100 + LAPSED_WINDOW_SECS + 5)
        .unwrap();
    assert_eq!(rows(&s), 0);
}

#[test]
fn housekeeping_keeps_a_lapse_row_inside_the_window() {
    let s = store();
    take(&s, ("r", "bob", "job"), 100, 0);
    take(&s, ("r", "alice", "x"), 50_000, 150);
    s.prune_members(1_000_000, 200).unwrap();
    assert_eq!(rows(&s), 1);
}
