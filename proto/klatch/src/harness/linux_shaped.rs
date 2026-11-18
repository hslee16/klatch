//! Session discovery and the liveness check against a process table shaped like Linux `ps` output (#171).
//!
//! **Constructed, not observed.** Every value here comes from knowledge of procps `ps` and of the registry format
//! seen on macOS; nothing was read from a Linux machine. What these tests pin is that the code copes with the shapes
//! Linux is known to print (a padded `lstart` day, a bare `comm` cut to 15 characters, state letters such as `Ssl`
//! and `I`, `HH:MM:SS` CPU times, a `linux` pid domain). What a real Linux registry and `ps` show is ticket #797.

use crate::harness::ancestry::{Miss, find_session};
use crate::harness::host::parse_ps_rows;
use crate::harness::liveness::fake::FakeTable;
use crate::harness::liveness::{Liveness, check, is_claude_code, pid_domain_for};
use crate::harness::record::{Record, Registry, parse_record};
use std::collections::HashMap;

/// The start time as `PsTable` hands it over (`TZ=UTC`, whitespace collapsed).
const START: &str = "Fri Nov 6 03:04:05 2026";

/// A registry with the records parsed from Linux-style JSON.
#[derive(Default)]
struct Reg(HashMap<u32, Record>);

impl Reg {
    fn with(mut self, json: &str) -> Reg {
        let rec = parse_record(json).expect("a well formed record");
        self.0.insert(rec.pid, rec);
        self
    }
}

impl Registry for Reg {
    fn record_for(&self, pid: u32) -> Option<Record> {
        self.0.get(&pid).cloned()
    }
}

/// What a Linux registry record is expected to look like: the `lstart` of procps pads a one digit day with a space,
/// and the pid domain is named for the system.
fn record(pid: u32) -> String {
    format!(
        r#"{{"pid":{pid},"sessionId":"lx-{pid}","cwd":"/home/dev/app","name":"lin","procStart":"Fri Nov  6 03:04:05 2026","pidDomain":"linux"}}"#
    )
}

/// systemd -> sshd -> sshd -> bash -> claude -> bash -> klatch, the shape of a coding session over ssh.
fn tree() -> FakeTable {
    FakeTable::default()
        .with((1, 0), "Thu Nov 5 00:00:00 2026", "systemd")
        .with((800, 1), "Thu Nov 5 00:00:10 2026", "sshd")
        .with((900, 800), "Fri Nov 6 03:00:00 2026", "sshd")
        .with((901, 900), "Fri Nov 6 03:00:01 2026", "bash")
        .with((910, 901), START, "claude")
        .with((920, 910), "Fri Nov 6 03:05:00 2026", "bash")
        .with((930, 920), "Fri Nov 6 03:05:01 2026", "klatch")
}

#[test]
fn the_pid_domain_is_darwin_for_macos_and_the_os_name_for_everything_else() {
    assert_eq!(pid_domain_for("macos"), "darwin");
    assert_eq!(pid_domain_for("linux"), "linux");
    assert_eq!(pid_domain_for("freebsd"), "freebsd");
}

#[test]
fn a_linux_record_with_a_padded_day_matches_the_squashed_ps_start_time() {
    let rec = parse_record(&record(910)).unwrap();
    assert_eq!(rec.proc_start, START, "the record is squashed when read");
    assert_eq!(check(&rec, &tree(), "linux"), Liveness::Live);
}

#[test]
fn a_linux_record_is_another_pid_space_on_macos_and_never_dead_or_live_there() {
    let rec = parse_record(&record(910)).unwrap();
    assert_eq!(check(&rec, &tree(), "darwin"), Liveness::OtherDomain);
    let empty = FakeTable::default();
    assert_eq!(check(&rec, &empty, "darwin"), Liveness::OtherDomain);
}

#[test]
fn the_walk_from_a_tool_up_a_ssh_session_tree_reaches_the_harness() {
    let reg = Reg::default().with(&record(910));
    let found = find_session(930, &tree(), &reg, "linux").unwrap();
    assert_eq!((found.session_id.as_str(), found.pid), ("lx-910", 910));
}

#[test]
fn the_walk_stops_at_init_and_a_record_for_a_reused_pid_is_stale() {
    let reused = tree().with((910, 901), "Sat Nov 7 09:00:00 2026", "claude");
    let reg = Reg::default().with(&record(910));
    assert_eq!(
        find_session(930, &reused, &reg, "linux"),
        Err(Miss::Stale(Liveness::Reused))
    );
    assert_eq!(
        find_session(930, &tree(), &Reg::default(), "linux"),
        Err(Miss::NoSession)
    );
}

#[test]
fn comm_is_cut_to_fifteen_characters_and_only_the_exact_name_is_the_harness() {
    // procps prints the kernel's comm: at most 15 characters, no directory
    assert!(is_claude_code("claude"));
    for other in ["claude-code-san", "claude-helper", "node", "claude-desktop"] {
        assert!(!is_claude_code(other), "{other}");
    }
    assert!(
        "claude-code-sandbox".len() > 15,
        "the cut is why it reads so"
    );
    let t = tree().with((910, 901), START, "claude-code-san");
    let rec = parse_record(&record(910)).unwrap();
    assert_eq!(check(&rec, &t, "linux"), Liveness::NotHarness);
}

#[test]
fn a_session_started_through_node_would_read_as_not_the_harness_until_a_linux_run_says_otherwise() {
    // an npm install of a script runs under node, whose comm is `node`: pinned as the present behaviour, and the
    // reason #797 asks for a real observation of both install kinds
    let t = tree().with((910, 901), START, "node");
    let rec = parse_record(&record(910)).unwrap();
    assert_eq!(check(&rec, &t, "linux"), Liveness::NotHarness);
}

#[test]
fn procps_rows_parse_with_their_state_letters_and_time_shapes() {
    let rows = parse_ps_rows(
        "    1     0 Ss   00:00:03\n    2     0 S    00:00:00\n   15     2 I    00:00:00\n  \
         910   901 Sl+  1-02:03:04\n  911   910 S<   00:12:30\n 4242   910 D    00:00:01\n 5000   910 Z    00:00:00\n",
    );
    let states: String = rows.iter().map(|r| r.state).collect();
    assert_eq!(states, "SSISSDZ");
    assert_eq!(rows[0].cpu_ms, 3_000);
    assert_eq!(rows[3].cpu_ms, ((24 + 2) * 60 + 3) * 60_000 + 4_000);
    assert_eq!(rows[4].cpu_ms, 12 * 60_000 + 30_000);
    assert_eq!((rows[2].pid, rows[2].ppid), (15, 2));
}
