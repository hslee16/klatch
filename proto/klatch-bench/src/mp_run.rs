//! The runner half of the multi-process fault tester (#625): real processes, real kills, then the reopen and the
//! checks. The pure parts (plan, ledger, judgement) are in `mp`.

use crate::mp::{Facts, Ledger, Op, Step, judge, plan, stray_files, warm_up};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// What one run of the tester does.
pub struct Config {
    pub seed: u64,
    pub rounds: usize,
    /// Processes running at the same time.
    pub workers: usize,
    /// Processes started in a round.
    pub ops: usize,
    /// Whether `rotate-key` is among the operations. Off, a rotation becomes a read (the seed still draws the same
    /// plan), which keeps known rotation bugs (#628, #629) from hiding the rest.
    pub rotate: bool,
}

/// What one round found.
pub struct RoundReport {
    pub round: usize,
    pub migrated: bool,
    pub started: usize,
    pub killed: usize,
    pub acked: usize,
    /// Everything that broke what klatch promises (empty when the round held).
    pub violations: Vec<String>,
    /// Failures of processes that were not killed, grouped by their first line of standard error.
    pub refusals: BTreeMap<String, usize>,
}

/// The store a round runs against: a directory of its own, removed afterwards unless the round found something.
struct Home {
    dir: PathBuf,
    keep: bool,
}

impl Drop for Home {
    fn drop(&mut self) {
        if !self.keep {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }
}

/// The room every post and claim goes to.
const ROOM: &str = "r";

fn klatch(bin: &Path, home: &Path, who: &str, now: Option<i64>, args: &[&str]) -> Command {
    let mut c = Command::new(bin);
    c.env_clear()
        .env("KLATCH_HOME", home)
        .env("KLATCH_KEY_DIR", home.join("keys"))
        .env("KLATCH_NAME", who)
        .env("KLATCH_ADMIN_TEST_TERMINAL", "1")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(t) = now {
        c.env("KLATCH_NOW", t.to_string());
    }
    c
}

/// Run to the end, as the checker does when no worker is left.
fn finish(bin: &Path, home: &Path, args: &[&str]) -> Result<String, String> {
    use std::io::Write;
    let mut child = klatch(bin, home, "checker", None, args)
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin);
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(first_line(&String::from_utf8_lossy(&out.stderr)))
    }
}

fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or("(no message)").trim();
    scrub_home(line).chars().take(240).collect()
}

/// The round's directory name differs on every run; naming it `<home>` lets the same failure group together.
fn scrub_home(line: &str) -> String {
    let tmp = std::env::temp_dir().to_string_lossy().into_owned();
    let mut out = String::new();
    let mut rest = line;
    while let Some(at) = rest.find(&tmp) {
        out.push_str(&rest[..at]);
        let tail = &rest[at + tmp.len()..];
        let dir_end = tail
            .trim_start_matches('/')
            .find('/')
            .map_or(tail.len(), |i| {
                i + tail.len() - tail.trim_start_matches('/').len()
            });
        out.push_str("<home>");
        rest = &tail[dir_end..];
    }
    out.push_str(rest);
    out
}

fn archive_files(home: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(home.join("archive"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "db"))
        .collect();
    files.sort();
    files
}

/// A process that is running, and what the ledger should say about it when it ends.
struct Running {
    child: Child,
    op: Op,
    kill_at: Option<Instant>,
    killed: bool,
}

fn text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Posts of the warm-up are stamped here; an archive made two days later with a retention of one day moves them.
const OLD: i64 = 1_800_000_000;
const DAY: i64 = 86_400;

/// The command line of one step, and the clock it needs. Messages of the warm-up are old by the time of an archive;
/// later posts are stamped days after the archive's cutoff, so they are never due (a real post is never older than
/// the cutoff). Each archive has a second of its own because an archive file is named by its time.
fn argv(home: &Path, i: usize, op: &Op, warm: usize) -> (Vec<String>, Option<i64>) {
    let room = ROOM.to_string();
    let words = |w: &[&str]| w.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let peek = vec!["read".into(), room.clone(), "--peek".into()];
    match op {
        Op::Post(body) => {
            let at = if i < warm { OLD } else { OLD + 5 * DAY };
            (vec!["post".into(), room, body.clone()], Some(at))
        }
        Op::Claim(n) => (
            vec![
                "claim".into(),
                room,
                format!("f{n}"),
                "--ttl".into(),
                "60".into(),
            ],
            None,
        ),
        Op::Archive => {
            let mut a = words(&["admin", "archive", "--room"]);
            a.push(room);
            (a, Some(OLD + 2 * DAY + i as i64))
        }
        Op::RestoreArchive => {
            let files = archive_files(home);
            match files.get(i % files.len().max(1)) {
                Some(f) => (
                    vec!["admin".into(), "restore-archive".into(), text(f)],
                    None,
                ),
                None => (peek, None),
            }
        }
        Op::RotateKey => (words(&["admin", "rotate-key"]), None),
        Op::Backup => {
            let file = home.join(format!("bk-{i}.db"));
            (vec!["admin".into(), "backup".into(), text(&file)], None)
        }
        Op::Read => (peek, None),
    }
}

fn start(bin: &Path, home: &Path, i: usize, step: &Step, warm: usize) -> Result<Running, String> {
    use std::io::Write;
    if step.op == Op::Archive {
        // retention is per room and cheap to set again; doing it here keeps the plan self-contained
        let _ = finish(bin, home, &["admin", "retention", ROOM, "1"]);
    }
    let (args, now) = argv(home, i, &step.op, warm);
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut child = klatch(bin, home, &format!("w{}", i % 7), now, &words)
        .spawn()
        .map_err(|e| format!("could not start klatch: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let typed = if step.op == Op::RotateKey {
            "rotate"
        } else {
            ""
        };
        let _ = writeln!(stdin, "{typed}");
    }
    let kill_at = step
        .kill_after_ms
        .map(|ms| Instant::now() + Duration::from_millis(ms));
    Ok(Running {
        child,
        op: step.op.clone(),
        kill_at,
        killed: false,
    })
}

fn op_name(op: &Op) -> &'static str {
    match op {
        Op::Post(_) => "post",
        Op::Claim(_) => "claim",
        Op::Archive => "archive",
        Op::RestoreArchive => "restore-archive",
        Op::RotateKey => "rotate-key",
        Op::Backup => "backup",
        Op::Read => "read",
    }
}

/// Fold a finished process into the ledger and the refusal counts.
fn account(r: Running, ledger: &mut Ledger, report: &mut RoundReport) {
    let Running {
        child, op, killed, ..
    } = r;
    let (ok, stderr) = match child.wait_with_output() {
        Ok(o) => (
            o.status.success(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        ),
        Err(e) => (false, e.to_string()),
    };
    if stderr.contains("panicked") {
        report.violations.push(format!(
            "{} panicked: {}",
            op_name(&op),
            first_line(&stderr)
        ));
    }
    if killed {
        report.killed += 1;
    }
    if let Op::Post(b) = &op {
        // a post is owed only when it said so; a killed or refused one may or may not have landed
        if ok && !killed {
            ledger.acked.insert(b.clone());
        } else {
            ledger.maybe.insert(b.clone());
        }
    }
    // a claim held by someone else is the ordinary answer to contention, not a finding
    if !ok && !killed && !stderr.contains("is claimed by") {
        let key = format!("{} -> {}", op_name(&op), first_line(&stderr));
        *report.refusals.entry(key).or_default() += 1;
    }
}

/// Run the plan with at most `workers` processes at once, killing the ones the plan says. True when it had to give up.
fn drive(
    bin: &Path,
    home: &Path,
    steps: &[Step],
    cfg: &Config,
    ledger: &mut Ledger,
    report: &mut RoundReport,
) -> Result<bool, String> {
    let mut running: Vec<Running> = Vec::new();
    let mut next = 0;
    let deadline = Instant::now() + Duration::from_secs(120);
    let warm = warm_up(steps.len());
    while next < steps.len() || !running.is_empty() {
        // the barrier: every warm-up post has finished before the first archive may start
        let held = next == warm && !running.is_empty();
        while next < steps.len() && running.len() < cfg.workers && !held {
            running.push(start(bin, home, next, &steps[next], warm)?);
            report.started += 1;
            next += 1;
        }
        let mut i = 0;
        while i < running.len() {
            let done = matches!(running[i].child.try_wait(), Ok(Some(_)));
            let due = running[i].kill_at.is_some_and(|t| Instant::now() >= t);
            if !done && due {
                // SIGKILL: the process gets no chance to clean up, which is the point
                let _ = running[i].child.kill();
                running[i].killed = true;
            }
            if done || running[i].killed {
                account(running.swap_remove(i), ledger, report);
            } else {
                i += 1;
            }
        }
        if Instant::now() > deadline {
            for mut r in running.drain(..) {
                let _ = r.child.kill();
                r.killed = true;
                account(r, ledger, report);
            }
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(false)
}

/// Every file under `home`, as paths relative to it with `/`.
fn listing(home: &Path) -> Vec<String> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = format!("{prefix}{}", e.file_name().to_string_lossy());
            out.push(name.clone());
            if e.path().is_dir() {
                walk(&e.path(), &format!("{name}/"), out);
            }
        }
    }
    let mut out = Vec::new();
    walk(home, "", &mut out);
    out.sort();
    out
}

fn store_key_file(home: &Path) -> Result<PathBuf, String> {
    std::fs::read_dir(home.join("keys"))
        .map_err(|e| format!("no key directory: {e}"))?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            let n = p.file_name().map(|n| n.to_string_lossy().into_owned());
            n.is_some_and(|n| n.starts_with("store-") && n.ends_with(".key"))
        })
        .ok_or_else(|| "no store key file".to_string())
}

/// Open the store the way a user would, with its current key, and read what the checks need.
pub fn read_store(home: &Path) -> Result<Facts, String> {
    let key = std::fs::read(store_key_file(home)?).map_err(|e| e.to_string())?;
    let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    let c = rusqlite::Connection::open(home.join("rooms.db")).map_err(|e| e.to_string())?;
    c.pragma_update(None, "key", format!("x'{hex}'"))
        .map_err(|e| e.to_string())?;
    let mut facts = Facts::default();
    for pragma in ["PRAGMA integrity_check", "PRAGMA cipher_integrity_check"] {
        let mut st = c.prepare(pragma).map_err(|e| format!("{pragma}: {e}"))?;
        let lines = st
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| format!("{pragma}: {e}"))?;
        for line in lines.flatten().filter(|l| l != "ok") {
            facts.integrity.push(format!("{pragma}: {line}"));
        }
    }
    let mut st = c
        .prepare("SELECT room, seq, body, kind FROM messages")
        .map_err(|e| e.to_string())?;
    let rows = st
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    for (room, seq, body, kind) in rows.flatten() {
        facts.rooms.entry(room).or_default().push(seq);
        if kind.as_deref() == Some("archived") {
            facts.tombstones += 1;
        } else if let Some(b) = body {
            facts.bodies.push(b);
        }
    }
    Ok(facts)
}

/// Start from a plaintext prototype database, so the first processes race to migrate it.
fn seed_plaintext(home: &Path) -> Result<Vec<String>, String> {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../klatch/fixtures/store/prototype-v2.db");
    std::fs::copy(&fixture, home.join("rooms.db"))
        .map_err(|e| format!("{}: {e}", fixture.display()))?;
    Ok(vec!["hello from v2".to_string()])
}

/// Run one round: plan, drive, then reopen the store and judge it. Even rounds start from a plaintext database.
pub fn run_round(bin: &Path, cfg: &Config, round: usize) -> Result<RoundReport, String> {
    let dir = std::env::temp_dir().join(format!(
        "klatch-faults-{}-{}-{round}",
        std::process::id(),
        cfg.seed
    ));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut home = Home { dir, keep: false };
    let migrated = round.is_multiple_of(2);
    let mut ledger = Ledger::default();
    if migrated {
        ledger.seeded = seed_plaintext(&home.dir)?;
    } else {
        finish(bin, &home.dir, &["join", ROOM])?;
    }
    let mut report = RoundReport {
        round,
        migrated,
        started: 0,
        killed: 0,
        acked: 0,
        violations: Vec::new(),
        refusals: BTreeMap::new(),
    };
    let mut steps = plan(cfg.seed, round, cfg.ops);
    for s in steps
        .iter_mut()
        .filter(|s| !cfg.rotate && s.op == Op::RotateKey)
    {
        s.op = Op::Read;
    }
    let hung = drive(bin, &home.dir, &steps, cfg, &mut ledger, &mut report)?;
    report.acked = ledger.acked.len();
    if hung {
        report
            .violations
            .push("processes did not finish in 120 s (hung) and were killed".into());
    }
    verify(bin, &home.dir, &ledger, &mut report);
    home.keep = !report.violations.is_empty();
    if home.keep {
        report.violations.push(format!(
            "the store is kept for a look: {}",
            home.dir.display()
        ));
    }
    Ok(report)
}

/// After the workers: reopen, restore every archive, look for strays, then judge the store.
fn verify(bin: &Path, home: &Path, ledger: &Ledger, report: &mut RoundReport) {
    if let Err(e) = finish(bin, home, &["history", ROOM, "--last", "1"]) {
        report
            .violations
            .push(format!("the store does not reopen: {e}"));
        return;
    }
    for f in archive_files(home) {
        if let Err(e) = finish(bin, home, &["admin", "restore-archive", &text(&f)]) {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned());
            report.violations.push(format!(
                "archive {} cannot be restored: {e}",
                name.unwrap_or_default()
            ));
        }
    }
    for s in stray_files(&listing(home)) {
        report
            .violations
            .push(format!("a finished run left {s} behind"));
    }
    match read_store(home) {
        Ok(facts) => report.violations.extend(judge(&facts, ledger)),
        Err(e) => report
            .violations
            .push(format!("could not read the store: {e}")),
    }
}
