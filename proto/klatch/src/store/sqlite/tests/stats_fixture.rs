//! The fixtures of the usage report tests (#1377): a store with a known history, every member-written string of which
//! carries `TOPSECRET`, and the same store with real claim, checkpoint, status and routing traffic.

use super::*;
use crate::domain::StatsOptions;

pub(super) const D1: i64 = 1_795_305_600 + 36_000; // 2026-11-22 10:00 UTC
pub(super) const D2: i64 = 1_795_392_000 + 36_000; // 2026-11-23 10:00 UTC
pub(super) const NOW: i64 = D2 + 3600;

pub(super) fn exec(s: &SqliteStore, sql: &str) {
    s.conn.execute_batch(sql).unwrap();
}

/// One message row to insert: a public, unattributed, plain message unless a builder says otherwise.
struct Msg<'a> {
    room: &'a str,
    seq: i64,
    sender: &'a str,
    ts: i64,
    body: &'a str,
    kind: Option<&'a str>,
    attribution: &'a str,
    label: &'a str,
    payload: Option<&'a str>,
}

impl<'a> Msg<'a> {
    fn new((room, seq): (&'a str, i64), sender: &'a str, ts: i64, body: &'a str) -> Self {
        Msg {
            room,
            seq,
            sender,
            ts,
            body,
            kind: None,
            attribution: "unattributed",
            label: "public",
            payload: None,
        }
    }

    fn kind(self, kind: &'a str) -> Self {
        Msg {
            kind: Some(kind),
            ..self
        }
    }

    fn verified(self) -> Self {
        Msg {
            attribution: "verified",
            ..self
        }
    }

    fn label(self, label: &'a str) -> Self {
        Msg { label, ..self }
    }

    fn payload(self, payload: &'a str) -> Self {
        Msg {
            payload: Some(payload),
            ..self
        }
    }

    fn insert(&self, s: &SqliteStore) {
        s.conn
            .execute(
                "INSERT INTO messages(room,seq,mid,sender,member,body,ts,attribution,label,kind,payload) VALUES(?1,?2,?3,?4,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    self.room,
                    self.seq,
                    format!("{}-{}", self.room, self.seq),
                    self.sender,
                    self.body,
                    self.ts,
                    self.attribution,
                    self.label,
                    self.kind,
                    self.payload
                ],
            )
            .unwrap();
    }
}

fn bench_messages(s: &SqliteStore) {
    let msgs = [
        Msg::new(("lobby", 1), "alice", D1, "seeking: TOPSECRET-ask").verified(),
        Msg::new(
            ("lobby", 2),
            "bob",
            D1 + 30,
            "answer to #1: TOPSECRET-answer",
        )
        .kind("note"),
        Msg::new(("lobby", 3), "alice", D1 + 100, "closed #1 ok"),
        Msg::new(("lobby", 4), "carol", D2, "seeking: TOPSECRET-second")
            .payload("TOPSECRET-payload"),
        Msg::new(("lobby", 5), "bob", D2 + 5, "ready TOPSECRET").kind("note"),
        Msg::new(("dm.alice.bob", 1), "alice", D1 + 200, "hello TOPSECRET-dm"),
        Msg::new(("dm.alice.bob", 2), "bob", D1 + 210, "hi TOPSECRET-dm"),
        Msg::new(("vault", 1), "dave", D2 + 9, "TOPSECRET-vault").label("TOPSECRET-label"),
        Msg::new(
            ("sealedroom", 1),
            "erin",
            D2 + 20,
            "seeking: TOPSECRET-sealedask",
        )
        .label("sealed1"),
        Msg::new(("sealedroom", 2), "erin", D2 + 21, "TOPSECRET-sealed").label("sealed1"),
        // an archived tombstone is not a live message
        Msg::new(("lobby", 6), "alice", D1, "").kind("archived"),
    ];
    for m in &msgs {
        m.insert(s);
    }
}

/// The policy, the members and the rows of every table the report counts.
const BENCH_ROWS: &str = "
    UPDATE members SET status='done: TOPSECRET-status' WHERE name='carol';
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
    INSERT INTO hook_runs(member,event,last_ts,runs,last_bytes,total_bytes,max_bytes) VALUES
      ('alice','PostToolUse',2000000000,100,5,40000,900),('bob','PostToolUse',2000000000,50,5,5000,300),('alice','UserPromptSubmit',2000000000,20,5,2000,400);
    INSERT INTO credentials(member,token_hash,clearance,parent,issued,expires,revoked) VALUES
      ('w-1',x'00','internal','human',1000,9999999999,0),('w-2',x'01','internal','human',1000,9999999999,1),
      ('w-3',x'02','internal','human',1000,9999999999,0);
    INSERT INTO ask_scopes(room,seq,scope) VALUES('lobby',1,'TOPSECRET-scope');
    INSERT INTO ask_refs(room,seq,pos,kind,value,label,snippet) VALUES
      ('lobby',1,0,'issue','TOPSECRET-value','TOPSECRET-label','TOPSECRET-snippet'),
      ('lobby',1,1,'pr','TOPSECRET-value','TOPSECRET-label','TOPSECRET-snippet');";

/// A store with a known history; every member-written string carries `TOPSECRET`.
pub(super) fn bench() -> SqliteStore {
    let s = store();
    bench_messages(&s);
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
    exec(&s, BENCH_ROWS);
    s
}

fn req<'a>(who: &'a str, path: &'a str, ttl: i64) -> ClaimRequest<'a> {
    ClaimRequest::new(("sealedroom", who), path, ttl, "TOPSECRET why")
}

/// The bench plus real claim traffic in a room and by members whose names carry `TOPSECRET`: two claims on one
/// path where the second is refused and later granted (a handled conflict, 90 s), a release, a lapse taken over, an
/// anchor release and a session end. The history rows are the ones the claim paths wrote, not rows made by hand.
pub(super) fn bench_with_claims() -> SqliteStore {
    let s = bench();
    let t = D2 + 10_000;
    s.claim(&req("bob-TOPSECRET", "TOPSECRET/plan.rs", 5_000), t)
        .unwrap();
    s.claim(&req("amy-TOPSECRET", "TOPSECRET/plan.rs", 5_000), t + 10)
        .unwrap_err();
    s.claim(&req("amy-TOPSECRET", "TOPSECRET/plan.rs", 5_000), t + 20)
        .unwrap_err();
    s.release_at("sealedroom", "bob-TOPSECRET", "TOPSECRET/plan.rs", t + 70)
        .unwrap();
    s.claim(&req("amy-TOPSECRET", "TOPSECRET/plan.rs", 5_000), t + 100)
        .unwrap();
    s.claim(&req("cy-TOPSECRET", "ticket:TOPSECRET", 100), t + 200)
        .unwrap();
    s.claim(&req("dee-TOPSECRET", "ticket:TOPSECRET", 100), t + 400)
        .unwrap(); // lapsed under cy: takeover
    s.claim(
        &req("eve-TOPSECRET", "merge:TOPSECRET", 1_000).anchored("gh:TOPSECRET"),
        t + 500,
    )
    .unwrap();
    s.release_anchored("gh:TOPSECRET", t + 560).unwrap();
    s.release_member_claims("amy-TOPSECRET", t + 700).unwrap();
    // machine checkpoints (#1408): two for one member (the second replaces the first), then a last one for another
    let at = |m| ("lobby", m);
    let body = "turn ended; claims TOPSECRET/plan.rs; files TOPSECRET/plan.rs";
    s.put_machine_checkpoint(at("amy-TOPSECRET"), body, false, t + 710)
        .unwrap();
    s.put_machine_checkpoint(at("amy-TOPSECRET"), body, false, t + 720)
        .unwrap();
    s.put_machine_checkpoint(at("bob-TOPSECRET"), body, true, t + 730)
        .unwrap();
    // machine status (#1403): members in the lobby, three working writes, one idle and one parked
    let status = |m, class: &str, at| {
        s.join("lobby", m, at).unwrap();
        let text = format!("{class} (auto)");
        s.put_machine_status(
            crate::store::Seat::new("lobby", m),
            (&text, class),
            &["working (auto)", "idle (auto)", "parked (auto)"],
            (at, 60),
        )
        .unwrap();
    };
    status("amy-TOPSECRET", "working", t + 740);
    status("bob-TOPSECRET", "working", t + 741);
    status("cy-TOPSECRET", "working", t + 742);
    status("amy-TOPSECRET", "idle", t + 743);
    status("bob-TOPSECRET", "parked", t + 744);
    // ask routing (#1407): ask 1 routed to one member, taken, lapsed and escalated to a coordinator; ask 2 routed to
    // nobody and escalated with nobody to reach
    use crate::domain::RouteKind;
    s.route_ask(
        "lobby",
        1,
        &[("bob-TOPSECRET".into(), RouteKind::Capable)],
        t + 740,
    )
    .unwrap();
    s.route_ask("lobby", 2, &[], t + 741).unwrap();
    s.note_route_outcome("lobby", 1, "taken", 1, t + 750)
        .unwrap();
    s.note_route_outcome("lobby", 1, "lapsed", 1, t + 800)
        .unwrap();
    s.note_route_outcome("lobby", 1, "escalated", 1, t + 801)
        .unwrap();
    s.note_route_outcome("lobby", 2, "escalated", 0, t + 900)
        .unwrap();
    s
}

pub(super) fn opts() -> StatsOptions {
    StatsOptions {
        top: 10,
        now: NOW,
        ..StatsOptions::default()
    }
}

/// The count under `k` in a counts table, or -1 when the table has no such row.
pub(super) fn get(c: &[(String, i64)], k: &str) -> i64 {
    c.iter().find(|(n, _)| n == k).map_or(-1, |(_, n)| *n)
}

/// Asserts that every `(key, count)` is in the counts table.
#[track_caller]
pub(super) fn assert_counts(c: &[(String, i64)], want: &[(&str, i64)]) {
    for (k, n) in want {
        assert_eq!(get(c, k), *n, "count of {k} in {c:?}");
    }
}
