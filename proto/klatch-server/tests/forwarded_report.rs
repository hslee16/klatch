//! The ignored forwarded-header counts are shown while the server runs and kept across restarts (#1490, epic #13).
//! While it runs: a counts-only line on stderr, at most one per interval and only when the counts changed. Kept: one
//! `forwarded-ignored` audit row per interval (what was counted since the last row) and one at shutdown, written by a
//! timer, never by a request. Names and counts only: a header value never reaches stderr or the store. The servers
//! here run with `KLATCH_TEST_SHORT_LIMITS` (a debug-build knob), which makes the intervals 1 s (line) and 2 s (row).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

static N: AtomicU32 = AtomicU32::new(0);
/// The library reads its environment when this process opens the store to read the rows: one test at a time.
static ENV: Mutex<()> = Mutex::new(());
const LINE_SECS: u64 = 1;
const ROW_SECS: u64 = 2;
const SEEN: &str = "forwarded identity headers seen so far: ";
/// A header value that must never be shown or kept.
const SECRET: &str = "SECRETMARK-7f3a-203.0.113.9";

struct Dir(PathBuf);

impl Dir {
    fn new() -> Dir {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = PathBuf::from(format!("/tmp/kr{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["", "h"] {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(dir.join(sub))
                .unwrap();
        }
        Dir(dir)
    }
    fn home(&self) -> PathBuf {
        self.0.join("h")
    }
    /// The `forwarded-ignored` rows of the store, oldest first, as (target, detail).
    fn rows(&self) -> Vec<(String, String)> {
        let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::set_var("KLATCH_HOME", self.home());
            std::env::set_var("KLATCH_KEY_DIR", self.home().join("keys"));
            std::env::remove_var("KLATCH_NAME");
            std::env::remove_var("KLATCH_IDENTITY_FILE");
        }
        let store =
            klatch::member::custody::open_store(&klatch::member::db_path().unwrap()).unwrap();
        let gateway = klatch::gateway::Gateway::new(
            store,
            klatch::member::CliClock,
            klatch::gateway::AllowAll,
        );
        let mut all = gateway
            .admin_audit(&klatch::domain::Principal::unattributed("reader"), 500)
            .unwrap();
        all.reverse();
        all.into_iter()
            .filter(|r| r.action == "forwarded-ignored")
            .map(|r| (r.target, r.detail))
            .collect()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Server {
    child: Child,
    addr: SocketAddr,
    log: PathBuf,
}

impl Server {
    fn started(dir: &Dir) -> Server {
        let home = dir.home();
        let log = dir
            .0
            .join(format!("err{}.log", N.fetch_add(1, Ordering::SeqCst)));
        let child = Command::new(env!("CARGO_BIN_EXE_klatch-server"))
            .args(["http", "--listen", "127.0.0.1:0"])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &home)
            .env("KLATCH_HOME", &home)
            .env("KLATCH_KEY_DIR", home.join("keys"))
            .env("KLATCH_TEST_SHORT_LIMITS", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log)
                    .unwrap(),
            ))
            .spawn()
            .expect("start klatch-server");
        let mut server = Server {
            child,
            addr: "127.0.0.1:0".parse().unwrap(),
            log,
        };
        let until = Instant::now() + Duration::from_secs(60);
        while Instant::now() < until {
            let text = server.text();
            if let Some(rest) = text
                .rsplit("http listening on ")
                .next()
                .filter(|_| text.contains("http listening on "))
                && rest.contains('\n')
            {
                server.addr = rest.split_whitespace().next().unwrap().parse().unwrap();
                return server;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("the server never listened");
    }

    fn text(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// Everything on stderr so far, once `want` is in it (or a panic after the deadline).
    fn await_log(&self, want: &str) -> String {
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            let text = self.text();
            if text.contains(want) {
                return text;
            }
            assert!(Instant::now() < until, "no {want:?} in:\n{text}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// SIGTERM, wait for the exit, and return all of stderr.
    fn stop(mut self) -> String {
        unsafe {
            libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM);
        }
        let until = Instant::now() + Duration::from_secs(20);
        while self.child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < until, "the server did not stop");
            std::thread::sleep(Duration::from_millis(20));
        }
        self.text()
    }
}

/// One request that carries an ignored header (with `SECRET` as its value); it is refused (no credential).
fn send_ignored(addr: SocketAddr, name: &str) {
    let mut s = TcpStream::connect(addr).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let head = format!("GET /v1/stream HTTP/1.1\r\nHost: localhost\r\n{name}: {SECRET}\r\n\r\n");
    let _ = s.write_all(head.as_bytes());
    let mut line = String::new();
    BufReader::new(s.try_clone().unwrap())
        .read_line(&mut line)
        .expect("a status line");
    assert!(line.contains("401"), "{line}");
    let _ = s.read_to_string(&mut String::new());
}

fn live_lines(log: &str) -> Vec<&str> {
    log.lines().filter(|l| l.contains(SEEN)).collect()
}

/// Sum of `requests=N` over the rows.
fn requests_in(rows: &[(String, String)]) -> u64 {
    rows.iter()
        .map(|(_, d)| {
            let n = d
                .split_whitespace()
                .next()
                .and_then(|w| w.strip_prefix("requests="));
            n.expect("a requests= first").parse::<u64>().unwrap()
        })
        .sum()
}

#[test]
fn a_live_line_shows_the_counts_when_they_change_and_not_again_while_they_stand() {
    let dir = Dir::new();
    let s = Server::started(&dir);
    send_ignored(s.addr, "X-Forwarded-For");
    send_ignored(s.addr, "X-Real-IP");
    let log = s.await_log(&format!("{SEEN}requests=2"));
    assert!(
        log.contains("x-forwarded-for=1") && log.contains("x-real-ip=1"),
        "{log}"
    );
    // the counts stand for several intervals: no second line
    std::thread::sleep(Duration::from_secs(LINE_SECS * 3 + 1));
    assert_eq!(live_lines(&s.text()).len(), 1, "{}", s.text());
    // a change brings a new line
    send_ignored(s.addr, "X-Forwarded-For");
    let log = s.await_log(&format!("{SEEN}requests=3"));
    assert_eq!(live_lines(&log).len(), 2, "{log}");
    s.stop();
}

#[test]
fn a_server_that_saw_none_writes_no_live_line_and_no_row() {
    let dir = Dir::new();
    let s = Server::started(&dir);
    std::thread::sleep(Duration::from_secs(ROW_SECS * 2 + 1));
    let log = s.stop();
    assert!(live_lines(&log).is_empty(), "{log}");
    assert!(dir.rows().is_empty());
}

#[test]
fn no_header_value_reaches_stderr_or_the_store() {
    let dir = Dir::new();
    let s = Server::started(&dir);
    send_ignored(s.addr, "X-Forwarded-User");
    send_ignored(s.addr, "X-SSL-Client-S-DN");
    s.await_log(&format!("{SEEN}requests=2"));
    // let a row be kept by the timer too, then stop (the shutdown row and summary follow)
    std::thread::sleep(Duration::from_secs(ROW_SECS + 1));
    let log = s.stop();
    for part in ["SECRETMARK", "7f3a", "203.0.113.9"] {
        assert!(!log.contains(part), "{part} leaked into:\n{log}");
    }
    let rows = dir.rows();
    assert!(!rows.is_empty());
    for (target, detail) in &rows {
        for part in ["SECRETMARK", "7f3a", "203.0.113.9"] {
            assert!(
                !target.contains(part) && !detail.contains(part),
                "{part} in a row: {detail}"
            );
        }
    }
    assert_eq!(requests_in(&rows), 2);
}

#[test]
fn the_counts_survive_a_restart_as_rows_that_add_up() {
    let dir = Dir::new();
    let first = Server::started(&dir);
    for _ in 0..3 {
        send_ignored(first.addr, "X-Forwarded-For");
    }
    first.stop();
    // stopped at once: the shutdown row alone holds the three
    assert_eq!(
        dir.rows(),
        [("".to_string(), "requests=3 x-forwarded-for=3".to_string())]
    );
    let second = Server::started(&dir);
    send_ignored(second.addr, "X-Real-IP");
    send_ignored(second.addr, "X-Real-IP");
    second.stop();
    let rows = dir.rows();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[1].1, "requests=2 x-real-ip=2");
    assert_eq!(requests_in(&rows), 5);
}

#[test]
fn a_flood_of_ignored_headers_cannot_drive_the_writes_faster_than_the_interval() {
    let dir = Dir::new();
    let s = Server::started(&dir);
    let begun = Instant::now();
    let mut sent = 0;
    while begun.elapsed() < Duration::from_secs(ROW_SECS * 3 + 1) {
        for _ in 0..40 {
            send_ignored(s.addr, "X-Forwarded-For");
            sent += 1;
        }
    }
    let elapsed = begun.elapsed().as_secs();
    let log = s.stop();
    let rows = dir.rows();
    // one row per interval at most, plus the one at shutdown
    let most_rows = (elapsed / ROW_SECS + 1) as usize;
    assert!(sent > 100, "a flood: {sent}");
    assert!(
        !rows.is_empty() && rows.len() <= most_rows,
        "{} rows for {elapsed}s: {rows:?}",
        rows.len()
    );
    assert!(rows.len() < sent / 10, "far fewer rows than requests");
    assert_eq!(
        requests_in(&rows),
        sent as u64,
        "nothing lost, nothing double counted"
    );
    let most_lines = (elapsed / LINE_SECS) as usize;
    assert!(
        live_lines(&log).len() <= most_lines,
        "{} lines for {elapsed}s",
        live_lines(&log).len()
    );
}
