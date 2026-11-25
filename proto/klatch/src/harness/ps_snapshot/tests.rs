use super::*;
use crate::harness::ancestry::{Miss, find_session, unregistered_agent_below};
use crate::harness::liveness::{Liveness, fake::FakeTable};
use crate::harness::record::{Record, Registry};
use std::os::unix::fs::PermissionsExt;
use std::time::Instant;

const T: &str = "Fri Nov 6 03:04:05 2026";
const LIMITS: Limits = Limits {
    timeout: Duration::from_secs(5),
    cap: 1 << 20,
};

/// tool (30) -> shell (20) -> claude (10) -> terminal (5) -> launchd (1), as `ps -A -o pid=,ppid=,lstart=,comm=`.
const TABLE: &str = "    1     0 Thu Nov  5 00:00:00 2026 launchd
    5     1 Fri Nov  6 01:00:00 2026 Terminal
   10     5 Fri Nov  6 03:04:05 2026 claude
   20    10 Sat Nov  7 09:59:59 2026 /bin/zsh
   30    20 Sat Nov  7 10:00:00 2026 /Applications/My Tool/klatch
";

fn fake_table() -> FakeTable {
    FakeTable::default()
        .with((1, 0), "Thu Nov 5 00:00:00 2026", "launchd")
        .with((5, 1), "Fri Nov 6 01:00:00 2026", "Terminal")
        .with((10, 5), T, "claude")
        .with((20, 10), "Sat Nov 7 09:59:59 2026", "/bin/zsh")
        .with(
            (30, 20),
            "Sat Nov 7 10:00:00 2026",
            "/Applications/My Tool/klatch",
        )
}

fn snapshot() -> Snapshot {
    Snapshot(Some(parse_table(TABLE).expect("the table parses")))
}

struct Reg(Vec<Record>);
impl Registry for Reg {
    fn record_for(&self, pid: u32) -> Option<Record> {
        self.0.iter().find(|r| r.pid == pid).cloned()
    }
}
fn rec(pid: u32, start: &str) -> Record {
    Record {
        pid,
        session_id: format!("s{pid}"),
        cwd: "/w".into(),
        name: None,
        proc_start: start.into(),
        pid_domain: "darwin".into(),
    }
}

fn dir(tag: &str) -> crate::store::test_dir::TestDir {
    crate::store::test_dir::TestDir::new(&format!("ps-{tag}"))
}

/// A `ps` made of a shell script, run as `/bin/sh SCRIPT` so a freshly written executable is never exec'd (a fork in
/// another test thread can hold it open and fail the exec with "text file busy").
fn fake_ps(d: &std::path::Path, body: &str) -> PsAt {
    let script = d.join("fake-ps.sh");
    std::fs::write(&script, body).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o644)).unwrap();
    PsAt {
        program: Some(PathBuf::from("/bin/sh")),
        pre_args: vec![script.to_str().unwrap().into()],
        limits: LIMITS,
    }
}

fn printing_table(d: &std::path::Path) -> PsAt {
    fs_write(&d.join("table.txt"), TABLE);
    let counter = d.join("calls");
    let body = format!(
        "echo run >> '{}'\ncat '{}'\n",
        counter.display(),
        d.join("table.txt").display()
    );
    fake_ps(d, &body)
}

fn fs_write(p: &std::path::Path, text: &str) {
    std::fs::write(p, text).unwrap();
}

/// Run `f` on a thread and give up after a few seconds, so a blocked call fails the test instead of hanging it (#1616).
fn within_seconds<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(20))
        .expect("the ps call blocked")
}

#[test]
fn the_snapshot_parses_the_fields_and_keeps_a_program_name_with_spaces() {
    let table = parse_table(TABLE).unwrap();
    assert_eq!(table.len(), 5);
    let info = &table[&10];
    assert_eq!((info.ppid, info.start_utc.as_str()), (5, T));
    assert_eq!(info.command, "claude");
    assert_eq!(table[&30].command, "/Applications/My Tool/klatch");
    assert_eq!(table[&20].start_utc, "Sat Nov 7 09:59:59 2026");
}

#[test]
fn a_row_that_does_not_parse_makes_the_whole_table_unreadable() {
    assert_eq!(parse_table("garbage\n"), None);
    assert_eq!(
        parse_table("  7   x Fri Nov  6 03:04:05 2026 claude\n"),
        None
    );
    assert_eq!(parse_table("  7   1 Fri Nov  6 claude\n"), None);
    assert_eq!(parse_table(""), None, "an empty table is not a table");
}

#[test]
fn a_pid_in_the_snapshot_is_found_and_one_not_in_it_is_missing() {
    let s = snapshot();
    assert!(matches!(s.lookup(10), Lookup::Found(i) if i.command == "claude"));
    assert_eq!(s.lookup(11), Lookup::Missing);
    assert_eq!(Snapshot(None).lookup(10), Lookup::Unreadable);
}

#[test]
fn the_snapshot_gives_the_walk_results_of_the_per_pid_table() {
    let reg = Reg(vec![rec(10, T)]);
    let (a, b) = (fake_table(), snapshot());
    for start in [30, 20, 10, 5, 1, 99] {
        assert_eq!(
            find_session(start, &a, &reg, "darwin"),
            find_session(start, &b, &reg, "darwin"),
            "find_session from {start}"
        );
    }
    let found = find_session(30, &b, &reg, "darwin").unwrap();
    assert_eq!(found.pid, 10);
    assert_eq!(
        unregistered_agent_below(30, &found, &a),
        unregistered_agent_below(30, &found, &b)
    );
    // a record whose start time differs, and one whose process is not the harness, are stale in both
    for stale in [
        rec(10, "Mon Nov 2 08:00:00 2026"),
        rec(5, "Fri Nov 6 01:00:00 2026"),
    ] {
        let reg = Reg(vec![stale]);
        let want = find_session(30, &a, &reg, "darwin");
        assert!(matches!(
            want,
            Err(Miss::Stale(Liveness::Reused | Liveness::NotHarness))
        ));
        assert_eq!(find_session(30, &b, &reg, "darwin"), want);
    }
}

#[test]
fn a_walk_over_the_real_program_spawns_it_once() {
    let d = dir("once");
    let ps = printing_table(&d);
    let reg = Reg(vec![rec(10, T)]);
    let found = find_session(30, &ps, &reg, "darwin").expect("the session is found");
    assert_eq!(found.pid, 10);
    let calls = std::fs::read_to_string(d.join("calls")).unwrap();
    assert_eq!(
        calls.lines().count(),
        1,
        "one ps for the whole walk: {calls}"
    );
    assert_eq!(unregistered_agent_below(30, &found, &ps), None);
    let calls = std::fs::read_to_string(d.join("calls")).unwrap();
    assert_eq!(calls.lines().count(), 2, "and one for the next walk");
}

#[test]
fn a_ps_that_prints_nothing_or_fails_is_an_unreadable_table() {
    let d = dir("fail");
    assert_eq!(fake_ps(&d, "exit 1\n").lookup(10), Lookup::Unreadable);
    assert_eq!(
        fake_ps(&d, "echo '  1 0 bad'\n").lookup(10),
        Lookup::Unreadable
    );
    assert_eq!(
        fake_ps(&d, "printf '\\377\\376'\n").lookup(10),
        Lookup::Unreadable
    );
    // a failing exit status is unreadable even when it printed good rows
    let body = format!("printf '%s' '{TABLE}'\nexit 1\n");
    assert_eq!(fake_ps(&d, &body).lookup(10), Lookup::Unreadable);
}

#[test]
fn a_program_that_cannot_start_is_an_unreadable_table() {
    let ps = PsAt {
        program: Some(PathBuf::from("/nonexistent/ps")),
        pre_args: vec![],
        limits: LIMITS,
    };
    assert_eq!(ps.lookup(10), Lookup::Unreadable);
}

#[test]
fn no_fixed_ps_is_no_process_table_and_never_a_search_of_path() {
    let none = PsAt {
        program: None,
        pre_args: vec![],
        limits: LIMITS,
    };
    assert_eq!(none.lookup(std::process::id()), Lookup::Unreadable);
    assert_eq!(fixed_ps_in(&["/nonexistent/ps", "/also/not/there"]), None);
    assert_eq!(fixed_ps_in(&["ps"]), None, "a bare name is never looked up");
    // a relative path to a real executable is not an absolute path either
    let relative = format!("{}bin/sh", "../".repeat(40));
    assert!(std::fs::metadata(&relative).is_ok(), "the fixture resolves");
    assert_eq!(fixed_ps_in(&[relative.as_str()]), None);
}

#[test]
fn the_fixed_ps_is_the_first_regular_executable_of_the_list() {
    let d = dir("fixed");
    let (plain, exe, dirx) = (d.join("plain"), d.join("exe"), d.join("dir"));
    fs_write(&plain, "x");
    std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).unwrap();
    fs_write(&exe, "x");
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::create_dir(&dirx).unwrap();
    let list = [
        dirx.to_str().unwrap(),
        plain.to_str().unwrap(),
        exe.to_str().unwrap(),
    ];
    assert_eq!(fixed_ps_in(&list), Some(exe.clone()));
    assert_eq!(
        fixed_ps_in(&list[..2]),
        None,
        "a directory or a non-executable is not ps"
    );
}

#[test]
fn this_system_has_a_fixed_ps() {
    let ps = fixed_ps_in(&PS_CANDIDATES).expect("a fixed ps exists on macOS and Linux");
    assert!(ps.is_absolute());
}

#[test]
fn the_child_gets_a_cleared_environment() {
    let d = dir("env");
    let ps = fake_ps(
        &d,
        &format!(
            "env > '{}'\ncat '{}'\n",
            d.join("env").display(),
            d.join("t").display()
        ),
    );
    fs_write(&d.join("t"), TABLE);
    assert!(matches!(ps.lookup(10), Lookup::Found(_)));
    let env = std::fs::read_to_string(d.join("env")).unwrap();
    for inherited in ["HOME=", "USER=", "PATH=", "KLATCH_"] {
        assert!(!env.contains(inherited), "{inherited} leaked: {env}");
    }
    assert!(env.contains("TZ=UTC"), "{env}");
    assert!(env.contains("LC_ALL=C"), "{env}");
}

#[test]
fn a_ps_that_hangs_is_killed_after_the_timeout_and_gives_unknown() {
    let d = dir("hang");
    let mut ps = fake_ps(
        &d,
        &format!("echo $$ > '{}'\nexec sleep 60\n", d.join("pid").display()),
    );
    ps.limits.timeout = Duration::from_millis(300);
    let started = Instant::now();
    let got = within_seconds(move || ps.lookup(10));
    assert_eq!(got, Lookup::Unreadable);
    assert!(started.elapsed() < Duration::from_secs(15));
    let pid: i32 = std::fs::read_to_string(d.join("pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // the child is gone (killed and reaped), not left running
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        -1,
        "the hung ps was left alive"
    );
}

#[test]
fn output_past_the_cap_is_unreadable_even_when_it_parses_and_exits_cleanly() {
    let d = dir("cap");
    fs_write(&d.join("t"), TABLE);
    let body = format!(
        "i=0\nwhile [ $i -lt 400 ]; do cat '{}' 2>/dev/null; i=$((i+1)); done\nexit 0\n",
        d.join("t").display()
    );
    let mut ps = fake_ps(&d, &body);
    // a cap on a row boundary: the cut output still parses, so only the cap refuses it
    ps.limits.cap = TABLE.len() * 3;
    assert_eq!(within_seconds(move || ps.lookup(10)), Lookup::Unreadable);
    // the same output under a roomy cap is a table: only the cap made it unreadable
    let mut roomy = fake_ps(&d, &body);
    roomy.limits.cap = 1 << 24;
    assert!(matches!(
        within_seconds(move || roomy.lookup(10)),
        Lookup::Found(_)
    ));
}

#[test]
fn a_pid_no_system_can_number_is_missing_without_running_ps() {
    let d = dir("limit");
    let ps = printing_table(&d);
    assert_eq!(ps.lookup(u32::MAX - 1), Lookup::Missing);
    assert!(!d.join("calls").exists(), "ps was not asked");
}

fn calls(d: &std::path::Path) -> usize {
    std::fs::read_to_string(d.join("calls"))
        .unwrap()
        .lines()
        .count()
}

#[test]
fn listing_sessions_and_finding_a_parent_each_spawn_ps_once() {
    let d = dir("others");
    let ps = printing_table(&d);
    let reg = Reg(vec![rec(10, T), rec(5, "Fri Nov 6 01:00:00 2026")]);
    let listing = Records(vec![rec(10, T), rec(20, "Sat Nov 7 09:59:59 2026")]);
    let found = crate::harness::list(&listing, &ps, "darwin");
    assert_eq!((found.live.len(), found.ignored.len()), (1, 1));
    assert_eq!(calls(&d), 1, "one ps for the whole listing");
    let session = find_session(30, &ps, &reg, "darwin").unwrap();
    let before = calls(&d);
    let _ = crate::harness::find_parent(&session, &ps, &reg, "darwin");
    assert_eq!(calls(&d), before + 1, "one ps for the parent walk");
}

struct Records(Vec<Record>);
impl Registry for Records {
    fn record_for(&self, pid: u32) -> Option<Record> {
        self.0.iter().find(|r| r.pid == pid).cloned()
    }
    fn records(&self) -> (Vec<Record>, usize) {
        (self.0.clone(), 0)
    }
}

#[test]
fn recording_a_codex_session_spawns_ps_once_for_the_walk_and_the_prune() {
    let d = dir("codex");
    let table = TABLE.replace(
        "   10     5 Fri Nov  6 03:04:05 2026 claude",
        "   10     5 Fri Nov  6 03:04:05 2026 codex",
    );
    let ps = printing_table(&d);
    fs_write(&d.join("table.txt"), &table);
    let out = d.join("out");
    std::fs::create_dir(&out).unwrap();
    std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o700)).unwrap();
    // two stale records for the prune to judge: each used to cost three more ps runs
    for pid in [20, 30] {
        let text = format!(
            r#"{{"pid":{pid},"sessionId":"x","cwd":"/w","procStart":"Mon Nov 2 08:00:00 2026","pidDomain":"darwin"}}"#
        );
        fs_write(&out.join(format!("{pid}.json")), &text);
        std::fs::set_permissions(
            &out.join(format!("{pid}.json")),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    let facts = crate::harness::SessionFacts {
        session_id: "s",
        cwd: "/w",
        name: None,
    };
    assert_eq!(
        crate::harness::write_session_record(&out, 30, &facts, (&ps, "darwin")),
        Ok(10)
    );
    assert_eq!(calls(&d), 1);
    assert!(
        !out.join("20.json").exists() && !out.join("30.json").exists(),
        "the stale ones were pruned"
    );
}
