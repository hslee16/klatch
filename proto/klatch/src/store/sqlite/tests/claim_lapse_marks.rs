//! Pins for the lapse record and the batched deadlock-mark read (#1757, follow-ups of #1620): the prune boundary of
//! `claim_lapses`, the window edges and room isolation of `note_lapsed_yield_ends`, its equivalence with the per-lapse
//! `note_yield_end` it replaced, and that the read is bounded by the lapses of the sweep, not by every mark stored.

use super::*;
use crate::domain::{LAPSED_WINDOW_SECS, YIELD_WINDOW_SECS};
use crate::store::sqlite::claim_lapses::Lapsed;

const W: i64 = LAPSED_WINDOW_SECS;
const NOW: i64 = 10_000;

fn take(s: &SqliteStore, (room, who, path): (&str, &str, &str), ttl: i64, now: i64) {
    s.claim(&ClaimRequest::new((room, who), path, ttl, ""), now)
        .unwrap();
}

fn lapse_row(s: &SqliteStore, path: &str, expires: i64) {
    s.conn
        .execute(
            "INSERT INTO claim_lapses(room,path,owner,expires,since) VALUES('r',?1,'bob',?2,0)",
            params![path, expires],
        )
        .unwrap();
}

fn lapse_paths(s: &SqliteStore) -> Vec<String> {
    s.rows("SELECT path FROM claim_lapses ORDER BY path", [], |r| {
        r.get(0)
    })
    .unwrap()
}

fn mark(s: &SqliteStore, room: &str, member: &str, ts: i64) {
    s.conn
        .execute(
            "INSERT OR REPLACE INTO claim_yields(room,member,path,ts,taker) VALUES(?1,?2,'',?3,'')",
            params![room, member, ts],
        )
        .unwrap();
}

fn yields(s: &SqliteStore) -> Vec<(String, String, String, i64, String)> {
    s.rows(
        "SELECT room,member,path,ts,taker FROM claim_yields ORDER BY room,member,path",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )
    .unwrap()
}

fn lapsed(room: &str, owner: &str, path: &str, expires: i64) -> Lapsed {
    Lapsed {
        passive: false,
        room: room.into(),
        path: path.into(),
        owner: owner.into(),
        scope: String::new(),
        held: 1,
        expires,
        since: 0,
    }
}

// the prune boundary of claim_lapses

#[test]
fn housekeeping_deletes_a_lapse_row_ending_exactly_a_window_ago_and_keeps_the_next_second() {
    let s = store();
    lapse_row(&s, "edge", NOW - W);
    lapse_row(&s, "inside", NOW - W + 1);
    s.prune_members(1_000_000, NOW).unwrap();
    assert_eq!(lapse_paths(&s), ["inside"]);
}

#[test]
fn the_sweep_deletes_a_lapse_row_ending_exactly_a_window_ago_and_keeps_the_next_second() {
    let s = store();
    lapse_row(&s, "edge", NOW - W);
    lapse_row(&s, "inside", NOW - W + 1);
    take(&s, ("r", "alice", "x"), 50_000, NOW);
    assert_eq!(lapse_paths(&s), ["inside"]);
}

// the window edges of the batched mark read, through a real sweep

/// bob's claim `job` lapses at 100; a mark at `ts`; a sweep at 150. Whether `job` was marked as ended.
fn marked_after_sweep(ts: i64) -> bool {
    let s = store();
    take(&s, ("r", "bob", "job"), 100, 0);
    mark(&s, "r", "bob", ts);
    take(&s, ("r", "alice", "x"), 50_000, 150);
    yields(&s).iter().any(|y| y.2 == "job")
}

#[test]
fn a_mark_exactly_a_window_before_the_lease_end_marks_the_lapse() {
    assert!(marked_after_sweep(100 - YIELD_WINDOW_SECS));
}

#[test]
fn a_mark_a_second_before_that_does_not() {
    assert!(!marked_after_sweep(100 - YIELD_WINDOW_SECS - 1));
}

#[test]
fn a_mark_exactly_at_the_lease_end_marks_the_lapse() {
    assert!(marked_after_sweep(100));
}

#[test]
fn a_mark_a_second_after_the_lease_end_does_not() {
    assert!(!marked_after_sweep(101));
}

// room and member isolation

#[test]
fn a_mark_in_one_room_never_marks_a_lapse_in_another() {
    let s = store();
    take(&s, ("b", "bob", "job"), 100, 0);
    mark(&s, "a", "bob", 50);
    take(&s, ("b", "alice", "x"), 50_000, 150);
    assert!(yields(&s).iter().all(|y| y.2 != "job"), "{:?}", yields(&s));
}

#[test]
fn a_mark_of_another_member_never_marks_a_lapse() {
    let s = store();
    take(&s, ("r", "bob", "job"), 100, 0);
    mark(&s, "r", "dave", 50);
    take(&s, ("r", "alice", "x"), 50_000, 150);
    assert!(yields(&s).iter().all(|y| y.2 != "job"));
}

#[test]
fn a_standing_overwrite_row_is_not_a_mark() {
    // only a row with an empty path is a deadlock mark
    let s = store();
    take(&s, ("r", "bob", "job"), 100, 0);
    s.conn
        .execute(
            "INSERT INTO claim_yields(room,member,path,ts,taker) VALUES('r','bob','other',50,'')",
            [],
        )
        .unwrap();
    take(&s, ("r", "alice", "x"), 50_000, 150);
    assert!(yields(&s).iter().all(|y| y.2 != "job"));
}

// the batched read against the per-lapse read it replaced

/// A small deterministic generator, so a failing seed can be replayed.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

#[test]
fn the_batched_read_ends_the_same_yields_as_the_per_lapse_read_on_random_data() {
    const ROOMS: [&str; 3] = ["r1", "r2", "r3"];
    const MEMBERS: [&str; 3] = ["m1", "m2", "m3"];
    for seed in 0..400u64 {
        let mut g = Lcg(seed);
        let (batched, per_lapse) = (store(), store());
        let mut lapses = Vec::new();
        for i in 0..(1 + g.next(8)) {
            let (room, owner) = (ROOMS[g.next(3) as usize], MEMBERS[g.next(3) as usize]);
            lapses.push(lapsed(
                room,
                owner,
                &format!("p{i}"),
                1000 + g.next(30) as i64,
            ));
        }
        for room in ROOMS {
            for member in MEMBERS {
                if g.next(3) == 0 {
                    continue;
                }
                // a mark on or next to the window edge of a random lapse, or anywhere
                let near = &lapses[g.next(lapses.len() as u64) as usize];
                let ts = match g.next(6) {
                    0 => near.expires - YIELD_WINDOW_SECS - 1,
                    1 => near.expires - YIELD_WINDOW_SECS,
                    2 => near.expires,
                    3 => near.expires + 1,
                    4 => near.expires - g.next(YIELD_WINDOW_SECS as u64) as i64,
                    _ => 500 + g.next(1200) as i64,
                };
                for s in [&batched, &per_lapse] {
                    mark(s, room, member, ts);
                    if g.next(2) == 0 {
                        s.conn
                            .execute(
                                "INSERT OR IGNORE INTO claim_yields(room,member,path,ts,taker) VALUES(?1,?2,'pre',?3,'t')",
                                params![room, member, ts],
                            )
                            .unwrap();
                    }
                }
            }
        }
        // the rows the generator wrote differ between stores only by the coin above; make them equal
        let rows = yields(&batched);
        per_lapse
            .conn
            .execute("DELETE FROM claim_yields", [])
            .unwrap();
        for (room, member, path, ts, taker) in rows {
            per_lapse
                .conn
                .execute(
                    "INSERT INTO claim_yields(room,member,path,ts,taker) VALUES(?1,?2,?3,?4,?5)",
                    params![room, member, path, ts, taker],
                )
                .unwrap();
        }
        batched.note_lapsed_yield_ends(&lapses).unwrap();
        for l in &lapses {
            per_lapse
                .note_yield_end(&l.room, (&l.owner, &l.path), l.expires)
                .unwrap();
        }
        assert_eq!(yields(&batched), yields(&per_lapse), "seed {seed}");
    }
}

// the read is bounded by the lapses, not by every mark stored

#[test]
fn the_mark_read_does_not_return_stale_marks_or_marks_of_other_rooms() {
    let candidates = |stale: i64| {
        let s = store();
        mark(&s, "r", "bob", 950);
        for i in 0..stale {
            // far outside any window of the lapse, and in rooms the sweep never lapsed
            mark(&s, "r", &format!("old{i}"), 1 + i);
            mark(&s, &format!("room{i}"), "bob", 950);
        }
        s.lapsed_marks(&[lapsed("r", "bob", "job", 1000)]).unwrap()
    };
    let (few, many) = (candidates(2), candidates(200));
    assert_eq!(few.len(), many.len(), "the read grew with the stale marks");
    assert_eq!(many, [("r".to_string(), "bob".to_string(), 950)]);
}

#[test]
fn the_mark_read_still_sees_marks_for_every_room_of_the_sweep() {
    let s = store();
    for room in ["a", "b", "c"] {
        mark(&s, room, "bob", 950);
    }
    let ls = [
        lapsed("a", "bob", "j1", 1000),
        lapsed("c", "bob", "j2", 1005),
    ];
    let mut got = s.lapsed_marks(&ls).unwrap();
    got.sort();
    let rooms: Vec<&str> = got.iter().map(|m| m.0.as_str()).collect();
    assert_eq!(rooms, ["a", "c"]);
}

/// `n` rooms, each with one lapse and one mark: every lapse must end as a yield.
fn many_rooms_all_marked(n: usize) {
    let s = store();
    let rooms: Vec<String> = (0..n).map(|i| format!("room{i}")).collect();
    let ls: Vec<Lapsed> = rooms
        .iter()
        .map(|r| lapsed(r, "bob", "job", 1000))
        .collect();
    for r in &rooms {
        mark(&s, r, "bob", 950);
    }
    s.note_lapsed_yield_ends(&ls).unwrap();
    let ended = yields(&s).iter().filter(|y| y.2 == "job").count();
    assert_eq!(ended, rooms.len());
}

#[test]
fn a_sweep_of_more_rooms_than_one_chunk_marks_each_lapse() {
    // 1501 rooms cross the 500-room chunk three times
    many_rooms_all_marked(1501);
}

#[test]
#[ignore = "about 6 s in a debug build: only the 32766 variable limit of one statement needs it; run with --ignored"]
fn a_sweep_of_more_rooms_than_sqlite_has_variables_marks_each_lapse() {
    many_rooms_all_marked(33_000);
}

#[test]
fn the_room_list_is_deduplicated_so_a_room_is_read_once_across_chunks() {
    // room0 has one lapse, the others two, so a room's two entries straddle the 500 boundary unless deduplicated
    let s = store();
    let mut ls = vec![lapsed("room0", "bob", "a", 1000)];
    for i in 1..600 {
        ls.push(lapsed(&format!("room{i}"), "bob", "a", 1000));
        ls.push(lapsed(&format!("room{i}"), "bob", "b", 1000));
    }
    for i in 0..600 {
        mark(&s, &format!("room{i}"), "bob", 950);
    }
    let got = s.lapsed_marks(&ls).unwrap();
    assert_eq!(got.len(), 600, "a mark was read more than once");
}

/// Run the batched and the per-lapse path over the same rows and compare the resulting yields.
fn same_yields(lapses: &[Lapsed], marks: &[(String, String, i64)], label: &str) {
    let (batched, per_lapse) = (store(), store());
    for s in [&batched, &per_lapse] {
        for (room, member, ts) in marks {
            mark(s, room, member, *ts);
        }
    }
    batched.note_lapsed_yield_ends(lapses).unwrap();
    for l in lapses {
        per_lapse
            .note_yield_end(&l.room, (&l.owner, &l.path), l.expires)
            .unwrap();
    }
    assert_eq!(yields(&batched), yields(&per_lapse), "{label}");
}

#[test]
fn the_batched_read_matches_the_per_lapse_read_across_chunk_boundaries_and_span_edges() {
    const MEMBERS: [&str; 3] = ["m0", "m1", "m2"];
    for n in [1usize, 2, 3, 7, 499, 500, 501, 1000, 1001] {
        for seed in 0..3u64 {
            let mut g = Lcg(seed * 1000 + n as u64);
            let mut lapses = Vec::new();
            for r in 0..n {
                for k in 0..(1 + g.next(2)) {
                    let owner = MEMBERS[g.next(3) as usize];
                    let expires = 2000 + g.next(5) as i64 * 100;
                    lapses.push(lapsed(&format!("r{r}"), owner, &format!("p{k}"), expires));
                }
            }
            let first = lapses.iter().map(|l| l.expires).min().unwrap();
            let last = lapses.iter().map(|l| l.expires).max().unwrap();
            let mut marks = Vec::new();
            for r in 0..n {
                for member in MEMBERS {
                    let ts = match g.next(9) {
                        0 => first - YIELD_WINDOW_SECS - 1,
                        1 => first - YIELD_WINDOW_SECS,
                        2 => first - YIELD_WINDOW_SECS + 1,
                        3 => last - 1,
                        4 => last,
                        5 => last + 1,
                        6 => first,
                        _ => first - YIELD_WINDOW_SECS + g.next((last - first + 600) as u64) as i64,
                    };
                    if g.next(3) != 0 {
                        marks.push((format!("r{r}"), member.to_string(), ts));
                    }
                }
            }
            // rooms with marks but no lapse, and a member marked in several rooms
            for r in 0..5 {
                marks.push((format!("extra{r}"), "m0".to_string(), last));
                marks.push((format!("extra{r}"), "m1".to_string(), first));
            }
            same_yields(&lapses, &marks, &format!("rooms {n} seed {seed}"));
        }
    }
}

#[test]
fn matching_marks_to_lapses_is_linear_not_lapses_times_marks() {
    // 10000 lapses over 5000 holders: a lapses x marks scan would examine 50 million marks (#1761)
    let s = store();
    let ls: Vec<Lapsed> = (0..10_000)
        .map(|i| lapsed("r", &format!("m{}", i % 5000), &format!("p{i}"), 1000))
        .collect();
    for i in 0..5000 {
        mark(&s, "r", &format!("m{i}"), 950);
    }
    crate::store::sqlite::claims::MATCH_WORK.with(|w| w.set(0));
    s.note_lapsed_yield_ends(&ls).unwrap();
    let work = crate::store::sqlite::claims::MATCH_WORK.with(|w| w.get());
    assert!(work <= 4 * (ls.len() + 5000), "examined {work} marks");
    assert_eq!(
        yields(&s).iter().filter(|y| !y.2.is_empty()).count(),
        10_000
    );
}
