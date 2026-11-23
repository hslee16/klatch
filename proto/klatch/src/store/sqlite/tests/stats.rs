//! The counts-only usage report (#1377): known content in, exact counts out, and no member-written text anywhere in
//! the output for any combination of options. The event sections are in `stats_events.rs`.

use super::stats_fixture::{D1, D2, assert_counts, bench, bench_with_claims, exec, get, opts};
use super::*;
use crate::domain::{RoomClass, StatsOptions, UsageStats, day_of};

/// `opts()` narrowed to a room, a first second and a number of senders.
fn narrowed(room: Option<&str>, since: Option<i64>, top: usize) -> StatsOptions {
    StatsOptions {
        room: room.map(Into::into),
        since,
        top,
        ..opts()
    }
}

#[test]
fn the_counts_match_the_known_history() {
    let r = bench().usage_stats(&opts()).unwrap();
    assert!(r.schema >= 37, "schema {}", r.schema);
    let total: i64 = r.rooms.iter().map(|x| x.messages).sum();
    assert_eq!(total, 10, "the archived tombstone is not a live message");
    assert_eq!(r.days, vec![(day_of(D1), 5), (day_of(D2), 5)]);
    assert_eq!(r.senders_distinct, 5);
    assert_eq!(r.senders_top[0].1, 3);
    assert_eq!(r.verified_senders, 1);
    assert_counts(&r.senders_top, &[("alice", 3)]);
    assert_counts(&r.attribution, &[("verified", 1), ("unattributed", 9)]);
    assert_counts(&r.kinds, &[("note", 2), ("(none)", 8)]);
}

#[test]
fn rooms_are_counted_and_classed() {
    let r = bench().usage_stats(&opts()).unwrap();
    let room = |n: &str| r.rooms.iter().find(|x| x.room == n).unwrap();
    let lobby = room("lobby");
    assert_eq!((lobby.messages, lobby.senders), (5, 3));
    assert_eq!((lobby.first_ts, lobby.last_ts), (D1, D2 + 5));
    let classes: Vec<RoomClass> = ["lobby", "dm.alice.bob", "vault"]
        .iter()
        .map(|n| room(n).class)
        .collect();
    assert_eq!(
        classes,
        [RoomClass::Ordinary, RoomClass::Dm, RoomClass::Private]
    );
}

#[test]
fn asks_are_counted_with_the_time_to_the_first_answer() {
    let a = bench().usage_stats(&opts()).unwrap().asks;
    assert_eq!((a.created, a.answered, a.closed), (2, 1, 1));
    assert_eq!(
        (a.first_answer_median, a.first_answer_p90),
        (Some(30), Some(30))
    );
    assert_eq!((a.with_scope, a.with_refs), (1, 1));
    assert_counts(&a.ref_kinds, &[("issue", 1), ("pr", 1)]);
    assert_eq!(a.sealed_unread, 2, "sealed messages are skipped, not read");
}

#[test]
fn claims_are_counted_by_class() {
    let c = bench().usage_stats(&opts()).unwrap().claims;
    // the lapsed claim is not live
    assert_counts(
        &c.live,
        &[("file", 1), ("ticket", 1), ("ask", 1), ("other", 1)],
    );
    assert_counts(&c.notes, &[("file", 2), ("ticket", 1)]);
    assert_counts(&c.waiting, &[("file", 1)]);
}

#[test]
fn members_refusals_audit_and_credentials_are_counted() {
    let r = bench().usage_stats(&opts()).unwrap();
    assert_counts(
        &r.refusals,
        &[("rate-limit", 2), ("scrub", 1), ("(other)", 1)],
    );
    let m = &r.members;
    let counts = (m.joined, m.never_posted, m.marked_done, m.muted);
    assert_eq!((counts, m.with_parent), ((6, 1, 1, 1), 1));
    assert_counts(&r.audit, &[("pause", 2), ("close", 1)]);
    assert_eq!((r.credentials_issued, r.credentials_revoked), (3, 1));
}

#[test]
fn hook_runs_and_the_bytes_they_injected_are_counted() {
    let r = bench().usage_stats(&opts()).unwrap();
    assert_counts(&r.hooks, &[("PostToolUse", 150), ("UserPromptSubmit", 20)]);
    // bytes injected (#1406): totals and the largest single run per event, the largest sessions first
    assert_counts(
        &r.hook_bytes,
        &[("PostToolUse", 45_000), ("UserPromptSubmit", 2_000)],
    );
    assert_counts(&r.hook_bytes_max, &[("PostToolUse", 900)]);
    assert_eq!(
        r.sessions_top_bytes,
        vec![("alice".to_string(), 42_000), ("bob".to_string(), 5_000)]
    );
}

#[test]
fn the_largest_sessions_are_limited_by_top_and_follow_since() {
    let s = bench();
    let one = s.usage_stats(&narrowed(None, None, 1)).unwrap();
    assert_eq!(one.sessions_top_bytes, vec![("alice".to_string(), 42_000)]);
    let none = s
        .usage_stats(&narrowed(None, Some(2_000_000_001), 10))
        .unwrap();
    assert!(none.hook_bytes.is_empty() && none.sessions_top_bytes.is_empty());
}

#[test]
fn a_store_from_before_the_byte_columns_reports_runs_and_no_bytes() {
    let s = bench();
    exec(
        &s,
        "ALTER TABLE hook_runs DROP COLUMN total_bytes; ALTER TABLE hook_runs DROP COLUMN max_bytes; ALTER TABLE hook_runs DROP COLUMN since_start;",
    );
    let r = s.usage_stats(&opts()).unwrap();
    assert_eq!(get(&r.hooks, "PostToolUse"), 150);
    assert!(
        r.hook_bytes.is_empty() && r.hook_bytes_max.is_empty() && r.sessions_top_bytes.is_empty()
    );
    assert!(r.to_text().contains("bytes injected"));
}

#[test]
fn bytes_injected_are_in_the_text_and_the_json_as_counts() {
    let r = bench().usage_stats(&opts()).unwrap();
    let text = r.to_text();
    for want in [
        "bytes injected into sessions by hook event",
        "PostToolUse",
        "45000",
        "largest sessions by bytes injected",
    ] {
        assert!(text.contains(want), "{want} missing:\n{text}");
    }
    let j = r.to_json();
    let first = |key: &str| (j[key][0]["name"].clone(), j[key][0]["count"].clone());
    assert_eq!(first("hook_bytes"), ("PostToolUse".into(), 45_000.into()));
    assert_eq!(j["hook_bytes_max"][0]["count"], 900);
    assert_eq!(first("sessions_top_bytes"), ("alice".into(), 42_000.into()));
}

#[test]
fn a_sealed_room_shows_counts_only() {
    let r = bench().usage_stats(&opts()).unwrap();
    let sealed = r.rooms.iter().find(|x| x.room == "sealedroom").unwrap();
    assert_eq!((sealed.messages, sealed.sealed, sealed.senders), (2, 2, 1));
    let lobby = r.rooms.iter().find(|x| x.room == "lobby").unwrap();
    assert_eq!(lobby.sealed, 0);
}

#[test]
fn room_narrows_the_counts() {
    let s = bench();
    let lobby = s.usage_stats(&narrowed(Some("lobby"), None, 10)).unwrap();
    assert_eq!(lobby.rooms.len(), 1);
    assert_eq!((lobby.senders_distinct, lobby.asks.created), (3, 2));
    assert_eq!(
        (get(&lobby.claims.live, "file"), lobby.members.joined),
        (1, 4)
    );
    let none = s.usage_stats(&narrowed(Some("nosuch"), None, 10)).unwrap();
    assert!(none.rooms.is_empty() && none.days.is_empty());
}

#[test]
fn since_narrows_the_counts() {
    let later = bench()
        .usage_stats(&narrowed(None, Some(D2 - 1), 10))
        .unwrap();
    assert_eq!(later.days, vec![(day_of(D2), 5)]);
    let asks = &later.asks;
    assert_eq!((asks.created, asks.answered), (1, 0));
    assert_eq!(asks.first_answer_median, None);
}

#[test]
fn top_limits_the_senders_listed() {
    let r = bench().usage_stats(&narrowed(None, None, 2)).unwrap();
    assert_eq!(r.senders_top.len(), 2);
    assert_eq!(r.senders_distinct, 5);
}

fn every_option() -> Vec<StatsOptions> {
    let mut all = vec![opts()];
    for room in ["lobby", "dm.alice.bob", "vault", "sealedroom", "nosuch"] {
        all.push(narrowed(Some(room), None, 10));
        all.push(narrowed(Some(room), Some(D2 - 1), 1));
    }
    all.push(narrowed(None, Some(D1), 1));
    all.push(narrowed(None, Some(D2 + 100), 10));
    all
}

/// Words a member wrote that must never appear in the report.
const NEEDLES: [&str; 8] = [
    "TOPSECRET",
    "plan.rs",
    "-TOPSECRET",
    "hello",
    "closed #1",
    "answer to",
    "seeking",
    "snippet",
];

#[test]
fn no_member_written_text_appears_for_any_combination_of_options() {
    for s in [bench(), bench_with_claims()] {
        for o in every_option() {
            let r: UsageStats = s.usage_stats(&o).unwrap();
            let outputs = [r.to_text(), r.to_json().to_string()];
            let leaked = NEEDLES
                .iter()
                .flat_map(|n| outputs.iter().map(move |out| (n, out)))
                .find(|(n, out)| out.contains(**n));
            assert!(leaked.is_none(), "{leaked:?} leaked with {o:?}");
        }
    }
}

#[test]
fn the_text_report_is_stable_and_names_its_sections() {
    let s = bench();
    let a = s.usage_stats(&opts()).unwrap().to_text();
    assert_eq!(a, s.usage_stats(&opts()).unwrap().to_text());
    for section in [
        "messages per room",
        "messages per day (UTC)",
        "top senders",
        "attribution",
        "message kinds",
        "time to first answer: median 30s, p90 30s",
        "live claims by class",
        "refusals by rule",
        "direct message rooms: 1 (2 messages); private rooms: 1 (1 messages)",
        "hook runs by event",
        "admin audit by act",
        "worker credentials: 3 issued, 1 revoked",
    ] {
        assert!(a.contains(section), "missing {section}:\n{a}");
    }
}

#[test]
fn a_store_file_is_read_without_changing_it() {
    let path = temp_db("usage-stats");
    let key = crate::store::insecure_test_key();
    {
        let s = SqliteStore::open(&path, &key).unwrap();
        s.post(Seat::new("r", "bob"), "TOPSECRET", D1).unwrap();
    }
    let before = std::fs::read(&path).unwrap();
    let r = SqliteStore::read_usage_stats(&path, &key, &opts()).unwrap();
    assert_eq!((r.rooms.len(), r.rooms[0].messages), (1, 1));
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "the report wrote to the store"
    );
    assert!(!r.to_text().contains("TOPSECRET"));
}

/// A store where alice, bob, carol and dave posted and erin only joined (#1411): bob read, dave was only shown.
fn never_read_store() -> SqliteStore {
    let s = store();
    let r = "lobby";
    s.post(Seat::new(r, "alice"), "TOPSECRET one", D1).unwrap();
    s.post(Seat::new(r, "bob"), "TOPSECRET two", D1 + 1)
        .unwrap();
    s.read(Seat::new(r, "bob"), true, D1 + 2).unwrap();
    // dave is shown the room by a hook digest but never reads
    s.do_hook_read(r, "dave", D2).unwrap();
    s.post(Seat::new(r, "carol"), "TOPSECRET three", D2 + 1)
        .unwrap();
    s.post(Seat::new(r, "dave"), "TOPSECRET four", D2 + 2)
        .unwrap();
    // erin joined and never posted: not counted as a poster
    s.join(r, "erin", D2).unwrap();
    s
}

/// #1411: members who posted and never read, by the day of their first post, counts only.
#[test]
fn members_who_posted_and_never_read_are_counted_by_day() {
    let m = never_read_store().usage_stats(&opts()).unwrap().members;
    assert_eq!(m.posted, 4, "alice, bob, carol, dave posted");
    assert_eq!(m.never_read, 3, "alice, carol and dave never read");
    assert_eq!(m.never_shown, 2, "dave's hook showed him the room");
    assert_eq!(
        m.never_read_days,
        vec![(day_of(D1), 1), (day_of(D2), 2)],
        "by the day of the first post"
    );
}

#[test]
fn the_never_read_lines_name_no_member() {
    let report = never_read_store().usage_stats(&opts()).unwrap();
    let (text, json) = (report.to_text(), report.to_json());
    for out in [text.clone(), json.to_string()] {
        assert!(!out.contains("TOPSECRET"), "{out}");
    }
    // the sender table names senders (it always has); the never-read lines and the members object do not
    let lines = text
        .lines()
        .filter(|l| l.contains("never read") || l.contains("first post"));
    let members = json["members"].to_string();
    let names = lines.chain([members.as_str()]);
    for line in names {
        assert!(!line.contains("alice") && !line.contains("carol"), "{line}");
    }
    assert!(text.contains("posted and never read"), "{text}");
}
