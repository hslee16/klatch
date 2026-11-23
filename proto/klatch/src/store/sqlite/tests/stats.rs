//! The counts-only usage report (#1377): known content in, exact counts out, and no member-written text anywhere in
//! the output for any combination of options.

use super::*;
use crate::domain::{RoomClass, StatsOptions, UsageStats, day_of};

const D1: i64 = 1_795_305_600 + 36_000; // 2026-11-22 10:00 UTC
const D2: i64 = 1_795_392_000 + 36_000; // 2026-11-23 10:00 UTC
const NOW: i64 = D2 + 3600;

fn exec(s: &SqliteStore, sql: &str) {
    s.conn.execute_batch(sql).unwrap();
}

#[allow(clippy::too_many_arguments)]
fn msg(
    s: &SqliteStore,
    room: &str,
    seq: i64,
    sender: &str,
    ts: i64,
    body: &str,
    kind: Option<&str>,
    attribution: &str,
    label: &str,
    payload: Option<&str>,
) {
    s.conn
        .execute(
            "INSERT INTO messages(room,seq,mid,sender,member,body,ts,attribution,label,kind,payload) VALUES(?1,?2,?3,?4,?4,?5,?6,?7,?8,?9,?10)",
            params![room, seq, format!("{room}-{seq}"), sender, body, ts, attribution, label, kind, payload],
        )
        .unwrap();
}

/// A store with a known history; every member-written string carries `TOPSECRET`.
fn bench() -> SqliteStore {
    let s = store();
    let p = "public";
    msg(
        &s,
        "lobby",
        1,
        "alice",
        D1,
        "seeking: TOPSECRET-ask",
        None,
        "verified",
        p,
        None,
    );
    msg(
        &s,
        "lobby",
        2,
        "bob",
        D1 + 30,
        "answer to #1: TOPSECRET-answer",
        Some("note"),
        "unattributed",
        p,
        None,
    );
    msg(
        &s,
        "lobby",
        3,
        "alice",
        D1 + 100,
        "closed #1 ok",
        None,
        "unattributed",
        p,
        None,
    );
    msg(
        &s,
        "lobby",
        4,
        "carol",
        D2,
        "seeking: TOPSECRET-second",
        None,
        "unattributed",
        p,
        Some("TOPSECRET-payload"),
    );
    msg(
        &s,
        "lobby",
        5,
        "bob",
        D2 + 5,
        "ready TOPSECRET",
        Some("note"),
        "unattributed",
        p,
        None,
    );
    msg(
        &s,
        "dm.alice.bob",
        1,
        "alice",
        D1 + 200,
        "hello TOPSECRET-dm",
        None,
        "unattributed",
        p,
        None,
    );
    msg(
        &s,
        "dm.alice.bob",
        2,
        "bob",
        D1 + 210,
        "hi TOPSECRET-dm",
        None,
        "unattributed",
        p,
        None,
    );
    msg(
        &s,
        "vault",
        1,
        "dave",
        D2 + 9,
        "TOPSECRET-vault",
        None,
        "unattributed",
        "TOPSECRET-label",
        None,
    );
    msg(
        &s,
        "sealedroom",
        1,
        "erin",
        D2 + 20,
        "seeking: TOPSECRET-sealedask",
        None,
        "unattributed",
        "sealed1",
        None,
    );
    msg(
        &s,
        "sealedroom",
        2,
        "erin",
        D2 + 21,
        "TOPSECRET-sealed",
        None,
        "unattributed",
        "sealed1",
        None,
    );
    // an archived tombstone is not a live message
    msg(
        &s,
        "lobby",
        6,
        "alice",
        D1,
        "",
        Some("archived"),
        "unattributed",
        p,
        None,
    );
    exec(
        &s,
        "INSERT INTO policy(key,value,ts) VALUES('label:vault','TOPSECRET-label',1),('sealed:sealed1','1',1);",
    );
    for (room, who) in [
        ("lobby", "alice"),
        ("lobby", "bob"),
        ("lobby", "carol"),
        ("vault", "dave"),
        ("sealedroom", "erin"),
        ("lobby", "frank"),
    ] {
        s.join(room, who, D1).unwrap();
    }
    exec(
        &s,
        "UPDATE members SET status='done: TOPSECRET-status' WHERE name='carol';
         UPDATE members SET muted=1 WHERE name='bob';
         INSERT INTO member_parent(member,parent,ts) VALUES('carol','alice',1);
         INSERT INTO claims(room,path,owner,member,reason,expires) VALUES
           ('lobby','src/a.rs','bob','bob','TOPSECRET-reason',9999999999),
           ('lobby','ticket:5','alice','alice','TOPSECRET-reason',9999999999),
           ('lobby','merge:main','alice','alice','TOPSECRET-reason',9999999999),
           ('lobby','ask:1','bob','bob','TOPSECRET-reason',9999999999),
           ('lobby','src/old.rs','bob','bob','TOPSECRET-reason',5);
         INSERT INTO claim_notes(room,path,owner,note,ts) VALUES
           ('lobby','src/a.rs','bob','TOPSECRET-note',2000000000),
           ('lobby','ticket:5','alice','TOPSECRET-note',2000000000),
           ('lobby','src/b.rs','bob','TOPSECRET-note',2000000000);
         INSERT INTO claim_waits(room,member,path,ts) VALUES('lobby','carol','src/a.rs',2000000000);
         INSERT INTO refusals(ts,member,room,operation,rule) VALUES
           (2000000000,'bob','lobby','post TOPSECRET','rate-limit'),
           (2000000001,'bob','lobby','post','rate-limit'),
           (2000000002,'carol','vault','claim','scrub'),
           (2000000003,'carol','lobby','x','TOPSECRET rule words');
         INSERT INTO admin_audit(ts,action,target,detail) VALUES
           (2000000000,'pause','TOPSECRET','TOPSECRET'),(2000000001,'pause','','TOPSECRET'),(2000000002,'close','lobby','');
         INSERT INTO hook_runs(member,event,last_ts,runs,last_bytes) VALUES
           ('alice','PostToolUse',2000000000,100,5),('bob','PostToolUse',2000000000,50,5),('alice','UserPromptSubmit',2000000000,20,5);
         INSERT INTO credentials(member,token_hash,clearance,parent,issued,expires,revoked) VALUES
           ('w-1',x'00','internal','human',1000,9999999999,0),('w-2',x'01','internal','human',1000,9999999999,1),
           ('w-3',x'02','internal','human',1000,9999999999,0);
         INSERT INTO ask_scopes(room,seq,scope) VALUES('lobby',1,'TOPSECRET-scope');
         INSERT INTO ask_refs(room,seq,pos,kind,value,label,snippet) VALUES
           ('lobby',1,0,'issue','TOPSECRET-value','TOPSECRET-label','TOPSECRET-snippet'),
           ('lobby',1,1,'pr','TOPSECRET-value','TOPSECRET-label','TOPSECRET-snippet');",
    );
    s
}

fn opts() -> StatsOptions {
    StatsOptions {
        top: 10,
        now: NOW,
        ..StatsOptions::default()
    }
}

fn get(c: &[(String, i64)], k: &str) -> i64 {
    c.iter().find(|(n, _)| n == k).map_or(-1, |(_, n)| *n)
}

#[test]
fn the_counts_match_the_known_history() {
    let r = bench().usage_stats(&opts()).unwrap();
    assert!(r.schema >= 37, "schema {}", r.schema);
    let total: i64 = r.rooms.iter().map(|x| x.messages).sum();
    assert_eq!(total, 10, "the archived tombstone is not a live message");
    let room = |n: &str| r.rooms.iter().find(|x| x.room == n).unwrap();
    assert_eq!(room("lobby").messages, 5);
    assert_eq!(room("lobby").senders, 3);
    assert_eq!(room("lobby").class, RoomClass::Ordinary);
    assert_eq!(room("dm.alice.bob").class, RoomClass::Dm);
    assert_eq!(room("vault").class, RoomClass::Private);
    assert_eq!(
        (room("lobby").first_ts, room("lobby").last_ts),
        (D1, D2 + 5)
    );
    assert_eq!(r.days, vec![(day_of(D1), 5), (day_of(D2), 5)]);
    assert_eq!(r.senders_distinct, 5);
    assert_eq!(get(&r.senders_top, "alice"), 3);
    assert_eq!(r.senders_top[0].1, 3);
    assert_eq!(get(&r.attribution, "verified"), 1);
    assert_eq!(get(&r.attribution, "unattributed"), 9);
    assert_eq!(r.verified_senders, 1);
    assert_eq!(get(&r.kinds, "note"), 2);
    assert_eq!(get(&r.kinds, "(none)"), 8);
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
    assert_eq!(get(&a.ref_kinds, "issue"), 1);
    assert_eq!(get(&a.ref_kinds, "pr"), 1);
    assert_eq!(a.sealed_unread, 2, "sealed messages are skipped, not read");
}

#[test]
fn claims_members_refusals_hooks_audit_and_credentials_are_counted() {
    let r = bench().usage_stats(&opts()).unwrap();
    assert_eq!(
        get(&r.claims.live, "file"),
        1,
        "the lapsed claim is not live"
    );
    assert_eq!(get(&r.claims.live, "ticket"), 1);
    assert_eq!(get(&r.claims.live, "ask"), 1);
    assert_eq!(get(&r.claims.live, "other"), 1);
    assert_eq!(get(&r.claims.notes, "file"), 2);
    assert_eq!(get(&r.claims.notes, "ticket"), 1);
    assert_eq!(get(&r.claims.waiting, "file"), 1);
    assert_eq!(get(&r.refusals, "rate-limit"), 2);
    assert_eq!(get(&r.refusals, "scrub"), 1);
    assert_eq!(get(&r.refusals, "(other)"), 1);
    let m = &r.members;
    assert_eq!(
        (
            m.joined,
            m.never_posted,
            m.marked_done,
            m.muted,
            m.with_parent
        ),
        (6, 1, 1, 1, 1)
    );
    assert_eq!(get(&r.hooks, "PostToolUse"), 150);
    assert_eq!(get(&r.hooks, "UserPromptSubmit"), 20);
    assert_eq!(get(&r.audit, "pause"), 2);
    assert_eq!(get(&r.audit, "close"), 1);
    assert_eq!((r.credentials_issued, r.credentials_revoked), (3, 1));
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
fn room_and_since_narrow_the_counts() {
    let s = bench();
    let lobby = s
        .usage_stats(&StatsOptions {
            room: Some("lobby".into()),
            ..opts()
        })
        .unwrap();
    assert_eq!(lobby.rooms.len(), 1);
    assert_eq!(lobby.senders_distinct, 3);
    assert_eq!(lobby.asks.created, 2);
    assert_eq!(get(&lobby.claims.live, "file"), 1);
    assert_eq!(lobby.members.joined, 4);
    let later = s
        .usage_stats(&StatsOptions {
            since: Some(D2 - 1),
            ..opts()
        })
        .unwrap();
    assert_eq!(later.days, vec![(day_of(D2), 5)]);
    assert_eq!(later.asks.created, 1);
    assert_eq!(later.asks.answered, 0);
    assert_eq!(later.asks.first_answer_median, None);
    let none = s
        .usage_stats(&StatsOptions {
            room: Some("nosuch".into()),
            ..opts()
        })
        .unwrap();
    assert!(none.rooms.is_empty() && none.days.is_empty());
}

#[test]
fn top_limits_the_senders_listed() {
    let r = bench()
        .usage_stats(&StatsOptions { top: 2, ..opts() })
        .unwrap();
    assert_eq!(r.senders_top.len(), 2);
    assert_eq!(r.senders_distinct, 5);
}

fn every_option() -> Vec<StatsOptions> {
    let mut all = vec![opts()];
    for room in ["lobby", "dm.alice.bob", "vault", "sealedroom", "nosuch"] {
        all.push(StatsOptions {
            room: Some(room.into()),
            ..opts()
        });
        all.push(StatsOptions {
            room: Some(room.into()),
            since: Some(D2 - 1),
            top: 1,
            ..opts()
        });
    }
    all.push(StatsOptions {
        since: Some(D1),
        top: 1,
        ..opts()
    });
    all.push(StatsOptions {
        since: Some(D2 + 100),
        ..opts()
    });
    all
}

#[test]
fn no_member_written_text_appears_for_any_combination_of_options() {
    let s = bench();
    for o in every_option() {
        let r: UsageStats = s.usage_stats(&o).unwrap();
        for out in [r.to_text(), r.to_json().to_string()] {
            for needle in [
                "TOPSECRET",
                "hello",
                "closed #1",
                "answer to",
                "seeking",
                "snippet",
            ] {
                assert!(!out.contains(needle), "{needle} leaked with {o:?}:\n{out}");
            }
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
    assert_eq!(r.rooms.len(), 1);
    assert_eq!(r.rooms[0].messages, 1);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "the report wrote to the store"
    );
    assert!(!r.to_text().contains("TOPSECRET"));
}
