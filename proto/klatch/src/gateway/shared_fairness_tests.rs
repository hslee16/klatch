//! What one slow request costs the other handles of a shared gateway (#1086): one owner thread serves one request at a
//! time. A measurement (ignored), and a test that a queued short request is served right after the slow one.

use super::shared::SharedGateway;
use crate::domain::Principal;
use crate::gateway::areas::*;
use crate::gateway::{AllowAll, Gateway, HistoryRange, SystemClock, Transport};
use crate::store::SqliteStore;
use crate::store::test_dir::temp_db;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const ROOM: &str = "r";

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

/// `n` posts into `room`, spread over members so the per-room receiver budget (100 posts a minute each) is not hit.
fn fill(gw: &SharedGateway, (room, tag): (&str, &str), n: usize, body: &str) {
    for m in 0..n.div_ceil(90) {
        let t = as_member(gw, &format!("{tag}{m}"));
        t.join(room).unwrap();
        for i in 0..90.min(n - m * 90) {
            t.post(room, &format!("{m}-{i} {body}")).unwrap();
        }
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// A short request issued while a slow one runs waits for it and no longer: it is served when the slow one ends, and
/// never starves (every probe completes). The slow request here is the biggest one a store call can be, a run end.
#[test]
fn a_short_request_queued_behind_a_slow_one_is_served_when_it_ends() {
    let (gw, _db) = shared("fair-queue");
    let admin = as_member(&gw, "admin");
    admin.admin_run_create("q").unwrap();
    admin.join("run.q.main").unwrap();
    fill(&gw, ("run.q.main", "f"), 2000, "x");
    let probe = as_member(&gw, "probe");
    probe.join(ROOM).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let flag = done.clone();
    let slow = std::thread::spawn(move || {
        let s = Instant::now();
        admin.admin_run_end("q").unwrap();
        flag.store(true, Ordering::SeqCst);
        s.elapsed()
    });
    let mut probes = 0;
    while !done.load(Ordering::SeqCst) {
        probe.who(ROOM).unwrap();
        probes += 1;
    }
    slow.join().unwrap();
    // A probe issued after the slow request returns at once.
    let s = Instant::now();
    probe.who(ROOM).unwrap();
    assert!(
        s.elapsed() < Duration::from_secs(2),
        "after the slow request the owner is free"
    );
    assert!(probes >= 1);
}

/// Run: `cargo test -p klatch --release --lib -- --ignored --nocapture shared_fairness` and read the printed lines.
#[test]
#[ignore = "measurement for #1086; prints timings"]
fn shared_fairness_measurement() {
    const BACKLOG: usize = 10_000;
    let (gw, _db) = shared("fair-measure");
    let admin = as_member(&gw, "admin");
    let run_room = "run.fair.main";
    admin.admin_run_create("fair").unwrap();
    admin.join(run_room).unwrap();
    let probe = as_member(&gw, "probe");
    probe.join(ROOM).unwrap();
    let bulk = as_member(&gw, "bulk");
    bulk.join(ROOM).unwrap();
    let t = Instant::now();
    fill(&gw, (run_room, "a"), BACKLOG, &"x".repeat(200));
    fill(&gw, (ROOM, "b"), BACKLOG, &"x".repeat(200));
    println!("fill: {} posts in {:?}", BACKLOG * 2, t.elapsed());

    print_alone(&probe);
    for (name, work) in slow_requests(&gw) {
        print_beside(&gw, name, work);
    }
}

type Slow = Box<dyn FnOnce() + Send>;

/// The short request on its own: 200 probes, the median and the worst.
fn print_alone(probe: &impl Transport) {
    let mut alone: Vec<f64> = (0..200)
        .map(|_| {
            let s = Instant::now();
            probe.who(ROOM).unwrap();
            ms(s.elapsed())
        })
        .collect();
    alone.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "short request alone: p50 {:.3} ms, max {:.3} ms",
        alone[100], alone[199]
    );
}

/// The slow requests to time a short request beside, by name.
fn slow_requests(gw: &SharedGateway) -> Vec<(&'static str, Slow)> {
    vec![
        ("history (capped at 200)", {
            let t = as_member(gw, "bulk");
            Box::new(move || {
                let range = HistoryRange {
                    since: None,
                    last: Some(100_000),
                };
                t.history(ROOM, range).unwrap();
            })
        }),
        ("read of a 10000 message backlog", {
            let t = as_member(gw, "bulk");
            Box::new(move || {
                t.read(ROOM, true).unwrap();
            })
        }),
        ("admin_audit(1000000)", {
            let t = as_member(gw, "admin");
            Box::new(move || {
                t.admin_audit(1_000_000).unwrap();
            })
        }),
        ("admin_run_end (delete 10000 messages)", {
            let t = as_member(gw, "admin");
            Box::new(move || {
                t.admin_run_end("fair").unwrap();
            })
        }),
    ]
}

/// Run `work` while a prober thread issues short requests, then print how long each took.
fn print_beside(gw: &SharedGateway, name: &str, work: Slow) {
    let done = Arc::new(AtomicBool::new(false));
    let stop = done.clone();
    let p = as_member(gw, "probe");
    let prober = std::thread::spawn(move || {
        let (mut worst, mut n) = (0.0f64, 0u32);
        while !stop.load(Ordering::SeqCst) {
            let s = Instant::now();
            p.who(ROOM).unwrap();
            worst = worst.max(ms(s.elapsed()));
            n += 1;
        }
        (worst, n)
    });
    std::thread::sleep(Duration::from_millis(50));
    let s = Instant::now();
    work();
    let took = ms(s.elapsed());
    done.store(true, Ordering::SeqCst);
    let (worst, n) = prober.join().unwrap();
    println!("{name}: {took:.1} ms; worst short request beside it {worst:.1} ms ({n} probes)");
}
