//! The measurements of #620. Run `turso-spike help`. Every command works in a directory you give it; nothing touches
//! the live klatch store or keychain (the SQLCipher side uses the public, worthless test key).
use klatch::store::{Seat, SqliteStore, Store, insecure_test_key};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;
use turso_spike::{Encryption, TursoStore};

const KINDS: &[&str] = &["sqlcipher", "turso", "turso-plain", "turso-mp"];

fn open(kind: &str, path: &Path) -> Box<dyn Store> {
    let p = path.to_str().unwrap();
    match kind {
        "sqlcipher" => {
            Box::new(SqliteStore::open(path, &insecure_test_key()).expect("open sqlcipher"))
        }
        "turso" => {
            Box::new(TursoStore::open_with_key(p, &insecure_test_key(), false).expect("open turso"))
        }
        "turso-mp" => Box::new(
            TursoStore::open_with_key(p, &insecure_test_key(), true).expect("open turso mp"),
        ),
        "turso-plain" => Box::new(TursoStore::open(p, None).expect("open turso plain")),
        _ => panic!("unknown kind {kind}"),
    }
}

fn arg(n: usize) -> String {
    std::env::args().nth(n).unwrap_or_else(|| usage())
}

fn usage() -> ! {
    eprintln!(
        "turso-spike bench <dir> <n> | race <dir> <kind> <procs> <posts> | inspect <dir> | migrate <dir>\nkinds: {KINDS:?}"
    );
    std::process::exit(2)
}

/// Mean microseconds per call of `f` over `n` calls.
fn mean_us(n: usize, mut f: impl FnMut(usize)) -> f64 {
    let t = Instant::now();
    for i in 0..n {
        f(i);
    }
    t.elapsed().as_secs_f64() * 1e6 / n as f64
}

fn bench(dir: &Path, n: usize) {
    println!(
        "{:<12} {:>10} {:>10} {:>10} {:>10} {:>8}",
        "backend", "post us", "read us", "wait us", "tail us", "db KiB"
    );
    for kind in KINDS {
        let path = dir.join(format!("bench-{kind}.db"));
        let s = open(kind, &path);
        let post = mean_us(n, |i| {
            s.post(
                Seat::new("r", "alice"),
                &format!("message number {i} with a little body text"),
                i as i64,
            )
            .unwrap();
        });
        // bob has read nothing: each read returns everything after his cursor, then advances it
        let read = mean_us(n, |i| {
            s.post(Seat::new("r", "alice"), "x", 1_000_000 + i as i64)
                .unwrap();
            s.read(Seat::new("r", "bob"), true, 1_000_000 + i as i64)
                .unwrap();
        }) - post;
        let wait = mean_us(n, |i| {
            s.wait_poll(Seat::new("r", "bob"), 2_000_000 + i as i64)
                .unwrap();
        });
        let tail = mean_us(n, |_| {
            s.tail("r", 0, 20).unwrap();
        });
        drop(s);
        let kib = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) / 1024
            + std::fs::metadata(format!("{}-wal", path.display()))
                .map(|m| m.len())
                .unwrap_or(0)
                / 1024;
        println!("{kind:<12} {post:>10.0} {read:>10.0} {wait:>10.0} {tail:>10.0} {kib:>8}");
    }
}

/// One worker process: post `posts` messages to room `r`, print `ok <n> err <m> <first error>`.
fn worker(kind: &str, path: &Path, id: usize, posts: usize) {
    // a failed open panics inside `open`; report the message on stdout for the parent to collect
    std::panic::set_hook(Box::new(|info| {
        let msg = info
            .payload()
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_default();
        println!("open-failed {}", msg.replace('\n', " "));
    }));
    let s = match std::panic::catch_unwind(|| open(kind, path)) {
        Ok(s) => s,
        Err(_) => return,
    };
    let (mut ok, mut err, mut first) = (0, 0, String::new());
    for i in 0..posts {
        match s.post(
            Seat::new("r", &format!("w{id}")),
            &format!("w{id} message {i}"),
            i as i64,
        ) {
            Ok(_) => ok += 1,
            Err(e) => {
                err += 1;
                if first.is_empty() {
                    first = e.to_string();
                }
            }
        }
    }
    println!("ok {ok} err {err} {first}");
}

fn race(dir: &Path, kind: &str, procs: usize, posts: usize) {
    let path = dir.join(format!("race-{kind}.db"));
    for ext in ["", "-wal", "-shm", "-log"] {
        let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
    }
    drop(open(kind, &path)); // create the file and schema once, so the workers race on data and not on creation
    let t = Instant::now();
    let exe = std::env::current_exe().unwrap();
    let kids: Vec<_> = (0..procs)
        .map(|i| {
            Command::new(&exe)
                .args([
                    "worker",
                    kind,
                    path.to_str().unwrap(),
                    &i.to_string(),
                    &posts.to_string(),
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    let (mut ok, mut err) = (0usize, 0usize);
    let mut samples = Vec::new();
    for k in kids {
        let out = k.wait_with_output().unwrap();
        let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let w: Vec<&str> = line.split_whitespace().collect();
        if w.len() >= 4 && w[0] == "ok" {
            ok += w[1].parse::<usize>().unwrap_or(0);
            let e = w[3].parse::<usize>().unwrap_or(0);
            err += e;
            if e > 0 {
                samples.push(w[4..].join(" "));
            }
        } else {
            samples.push(format!("worker said {line:?} exit {:?}", out.status.code()));
        }
    }
    let secs = t.elapsed().as_secs_f64();
    let s = open(kind, &path);
    let all = s.tail("r", 0, 1_000_000).unwrap();
    let dense = all.iter().enumerate().all(|(i, m)| m.seq == i as i64 + 1);
    let mut ids: Vec<_> = all.iter().map(|m| m.id.clone()).collect();
    ids.sort();
    ids.dedup();
    println!(
        "{kind:<12} procs {procs} posts/proc {posts}: wrote {ok} failed {err} stored {} head {} dense {dense} unique-ids {} in {secs:.2}s",
        all.len(),
        s.head("r").unwrap(),
        ids.len() == all.len()
    );
    samples.sort();
    samples.dedup();
    for e in samples.iter().take(3) {
        println!("    error sample: {e}");
    }
}

/// What a file looks like from outside: its first bytes, whether a known plaintext shows in the file or its WAL,
/// and whether the other engine can open it.
fn inspect(dir: &Path) {
    let marker = "PLAINTEXT-MARKER-QZX-98765";
    for kind in KINDS {
        let path = dir.join(format!("inspect-{kind}.db"));
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
        }
        let s = open(kind, &path);
        s.post(Seat::new("r", "alice"), marker, 1).unwrap();
        drop(s);
        let leak = |f: String| {
            std::fs::read(&f)
                .map(|b| b.windows(marker.len()).any(|w| w == marker.as_bytes()))
                .map(|x| x.to_string())
                .unwrap_or_else(|_| "no file".into())
        };
        let head = std::fs::read(&path)
            .map(|b| b[..16.min(b.len())].to_vec())
            .unwrap_or_default();
        println!(
            "{kind:<12} header {:?} marker in db: {} in wal: {}",
            String::from_utf8_lossy(&head).replace('\0', "."),
            leak(path.display().to_string()),
            leak(format!("{}-wal", path.display()))
        );
    }
    // cross-engine opens
    let t = dir.join("inspect-turso.db");
    let c = dir.join("inspect-sqlcipher.db");
    println!(
        "SQLCipher store opens the Turso (encrypted) file: {:?}",
        SqliteStore::open(&t, &insecure_test_key())
            .map(|_| ())
            .map_err(|e| e.to_string())
    );
    let tp = dir.join("inspect-turso-plain.db");
    println!(
        "SQLCipher store opens the Turso (plain) file: {:?}",
        SqliteStore::open(&tp, &insecure_test_key())
            .map(|_| ())
            .map_err(|e| e.to_string())
    );
    println!(
        "Turso opens the SQLCipher file (plain): {:?}",
        TursoStore::open(c.to_str().unwrap(), None)
            .map(|_| ())
            .map_err(|e| e.to_string())
    );
    println!(
        "Turso opens the SQLCipher file (encrypted): {:?}",
        TursoStore::open_with(
            c.to_str().unwrap(),
            Some(Encryption {
                cipher: "aegis256".into(),
                hexkey: "5a".repeat(32)
            }),
            false
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    );
    println!(
        "Turso (encrypted) with a wrong key: {:?}",
        TursoStore::open_with(
            t.to_str().unwrap(),
            Some(Encryption {
                cipher: "aegis256".into(),
                hexkey: "11".repeat(32)
            }),
            false
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    );
}

/// Migration from an SQLCipher store to a Turso one through the Store trait (messages and members only).
fn migrate(dir: &Path) {
    let src_path = dir.join("migrate-src.db");
    let dst_path = dir.join("migrate-dst.db");
    for p in [&src_path, &dst_path] {
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", p.display()));
        }
    }
    let src = open("sqlcipher", &src_path);
    for i in 0..500 {
        let room = if i % 2 == 0 { "a" } else { "b" };
        src.post(
            Seat::new(room, if i % 3 == 0 { "alice" } else { "bob" }),
            &format!("m{i}"),
            i,
        )
        .unwrap();
    }
    let dst = open("turso", &dst_path);
    let t = Instant::now();
    let mut n = 0;
    for room in ["a", "b"] {
        for m in src.tail(room, 0, 10_000).unwrap() {
            let msg =
                klatch::store::NewMessage::new((room, &m.sender), m.attribution, &m.label, &m.body);
            let posted = dst.post_as(&msg, m.ts).unwrap();
            assert_eq!(posted.seq, m.seq);
            n += 1;
        }
    }
    let same = ["a", "b"].iter().all(|r| {
        let a: Vec<_> = src
            .tail(r, 0, 10_000)
            .unwrap()
            .into_iter()
            .map(|m| (m.seq, m.sender, m.body))
            .collect();
        let b: Vec<_> = dst
            .tail(r, 0, 10_000)
            .unwrap()
            .into_iter()
            .map(|m| (m.seq, m.sender, m.body))
            .collect();
        a == b
    });
    println!(
        "migrated {n} messages in {:.2}s through the Store trait; seq, sender and body identical: {same}",
        t.elapsed().as_secs_f64()
    );
    println!("message ids are NOT preserved (post_as mints a new id), see the note");
}

fn main() {
    match arg(1).as_str() {
        "bench" => bench(Path::new(&arg(2)), arg(3).parse().unwrap()),
        "race" => race(
            Path::new(&arg(2)),
            &arg(3),
            arg(4).parse().unwrap(),
            arg(5).parse().unwrap(),
        ),
        "worker" => worker(
            &arg(2),
            Path::new(&arg(3)),
            arg(4).parse().unwrap(),
            arg(5).parse().unwrap(),
        ),
        "inspect" => inspect(Path::new(&arg(2))),
        "migrate" => migrate(Path::new(&arg(2))),
        _ => usage(),
    }
}
