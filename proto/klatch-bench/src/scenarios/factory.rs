use super::*;
use crate::world::WorkerLauncher;
use std::path::Path;
use std::time::Instant;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "fifty_credentialed_workers_post_claim_and_read_at_once",
        origin: "#124, #121, #118",
        layer: "scale",
        run: fifty_workers,
    }]
}

const WORKERS: usize = 50;
const POSTS: usize = 10;

/// The sequence numbers in `read` output, whose lines look like `[seq ts sender] body`.
fn seqs(output: &str) -> Vec<u64> {
    output
        .lines()
        .filter_map(|l| l.strip_prefix('[')?.split(' ').next()?.parse().ok())
        .collect()
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() as f64 - 1.0) * p) as usize]
}

/// Issues the credential bundles of all the workers into `dir`.
fn issue_workers(w: &World, dir: &str) -> Result<(), String> {
    let issued = w.run(
        "launcher",
        &[
            "worker",
            "issue",
            "--count",
            &WORKERS.to_string(),
            "--prefix",
            "w",
            "--clearance",
            "internal",
            "--parent",
            "factory",
            "--out-dir",
            dir,
        ],
    );
    check(
        issued.ok,
        format!("issuing {WORKERS} workers failed: {}", issued.stderr),
    )
}

/// What one worker does: join, post, contend for one path, take its own, and read. Returns the latencies of the posts
/// that succeeded, whether it won the contested path, and whether it took its own.
fn one_worker(launcher: &WorkerLauncher, bundle: &Path, i: usize) -> (Vec<f64>, bool, bool) {
    let mut post_ms = Vec::new();
    launcher.run(bundle, &["join", "run"]);
    for k in 0..POSTS {
        let t = Instant::now();
        let r = launcher.run(bundle, &["post", "run", &format!("worker {i} message {k}")]);
        if r.ok {
            post_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let contested = launcher
        .run(bundle, &["claim", "run", "shared/path", "--ttl", "600"])
        .ok;
    let own = launcher
        .run(
            bundle,
            &["claim", "run", &format!("own/{i}"), "--ttl", "600"],
        )
        .ok;
    launcher.run(bundle, &["read", "run", "--peek"]);
    (post_ms, contested, own)
}

/// No message lost or doubled, in a dense order, every one marked verified; each claim held once.
fn check_store_after_the_rush(w: &World) -> Result<(), String> {
    // read in as many calls as the cap needs (#1390); the order is checked as it comes, not after a sort
    let read = w.read_all("checker", "run")?;
    let s = seqs(&read);
    let want: Vec<u64> = (1..=(WORKERS * POSTS) as u64).collect();
    check(
        s == want,
        format!(
            "{} messages read, the sequence is not 1 to {}",
            s.len(),
            WORKERS * POSTS
        ),
    )?;
    lacks(
        &read,
        "(unverified)",
        "a credentialed worker's message was not marked verified",
    )?;
    let claims = w.ok("checker", &["claims", "run"])?;
    check(
        claims.lines().filter(|l| l.contains("shared/path")).count() == 1,
        "the contested path is held more than once",
    )?;
    check(
        claims.lines().filter(|l| l.contains("own/")).count() == WORKERS,
        "a private claim is missing",
    )
}

fn fifty_workers(w: &World) -> Res {
    w.set_clock(T0);
    let dir = w.home.join("bundles");
    let dir_s = dir.to_string_lossy().into_owned();
    issue_workers(w, &dir_s)?;
    let launcher = w.worker_launcher();
    let started = Instant::now();
    // every worker, at once: join, post, contend for one path, take its own, and read
    let results: Vec<(Vec<f64>, bool, bool)> = std::thread::scope(|s| {
        let handles: Vec<_> = (1..=WORKERS)
            .map(|i| {
                let launcher = launcher.clone();
                let bundle = dir.join(format!("w-{i}.bundle"));
                s.spawn(move || one_worker(&launcher, &bundle, i))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect()
    });
    let wall = started.elapsed().as_secs_f64();
    let mut post_ms: Vec<f64> = results.iter().flat_map(|r| r.0.clone()).collect();
    post_ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    check(
        post_ms.len() == WORKERS * POSTS,
        format!("{} of {} posts succeeded", post_ms.len(), WORKERS * POSTS),
    )?;
    // exactly one winner of the contested path, and every private path taken
    let winners = results.iter().filter(|r| r.1).count();
    check(
        winners == 1,
        format!("{winners} workers won one contested path"),
    )?;
    check(
        results.iter().all(|r| r.2),
        "a worker could not take a path of its own",
    )?;
    check_store_after_the_rush(w)?;
    Ok(vec![
        ("workers".into(), WORKERS as f64),
        ("wall_s".into(), wall),
        ("posts_per_s".into(), (WORKERS * POSTS) as f64 / wall),
        ("post_p50_ms".into(), percentile(&post_ms, 0.5)),
        ("post_p99_ms".into(), percentile(&post_ms, 0.99)),
    ])
}
