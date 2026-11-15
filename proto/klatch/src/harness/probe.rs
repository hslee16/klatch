//! Is the session behind a registry record working, waiting or gone? A probe that runs when someone asks (DESIGN
//! section 29, epic #308): no daemon, no timer. Layers, cheapest first: identity, process state, turn state,
//! progress. Every layer can be unreadable and then counts as unknown; unknown is never dead. The probe reads file
//! metadata only (size and modification time), never a file's content, and never the environment.
//!
//! The first sample has nothing to compare with, so it records a `Sample` for the second; the caller keeps it.

use crate::harness::liveness::{self, Liveness};
use crate::harness::process::ProcessTable;
use crate::harness::record::Record;

/// What the probe concludes. `Unknown` is for "cannot tell", which a lease rule must treat as alive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The process is not the session any more (no such pid, a reused pid, or not the harness).
    Gone,
    /// Idle: the turn ended and the session waits for a person.
    WaitingOnHuman,
    /// Busy, with progress in the window (or a turn that only just began).
    Working,
    /// Busy, quiet, with a live child process: a tool call is running.
    WaitingOnTool,
    /// Busy with no progress for a long time and no children, or a zombie or stopped process.
    Stuck,
    /// Not enough readable layers to say.
    Unknown,
}

/// The registry's turn state. Anything else (a newer harness) is unknown, not a third state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Busy,
    Idle,
}

/// The registry's `status` and `statusUpdatedAt`; each is `None` when missing or of a shape not understood.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Turn {
    pub status: Option<Status>,
    pub updated_ms: Option<u64>,
}

/// Size and modification time of a file; its content is never read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileMeta {
    pub len: u64,
    pub modified_ms: u64,
}

/// One row of the process table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcRow {
    pub pid: u32,
    pub ppid: u32,
    /// First letter of the `ps` state: R, S, I, U, Z (zombie), T (stopped).
    pub state: char,
    pub cpu_ms: u64,
}

/// Where the probe gets everything but the identity check, behind a trait so tests use made-up files and tables.
pub trait Host {
    fn now_ms(&self) -> u64;
    /// The whole table; `None` when it cannot be read.
    fn procs(&self) -> Option<Vec<ProcRow>>;
    /// The registry's turn state for this pid.
    fn turn(&self, pid: u32) -> Turn;
    /// The session transcript's metadata.
    fn transcript(&self, rec: &Record) -> Option<FileMeta>;
    /// Metadata of the newest background-task output file of the session.
    fn task_output(&self, rec: &Record) -> Option<FileMeta>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thresholds {
    /// Progress this recent counts as working.
    pub window_ms: u64,
    /// Busy with no progress for this long, and no children, is stuck.
    pub stuck_after_ms: u64,
    /// Process-tree CPU growth below this between two samples is noise.
    pub cpu_min_ms: u64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            window_ms: 120_000,
            stuck_after_ms: 600_000,
            cpu_min_ms: 100,
        }
    }
}

/// What one probe leaves for the next: the values to compare with, and when progress was last seen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sample {
    pub at_ms: u64,
    pub cpu_ms: Option<u64>,
    pub transcript_len: Option<u64>,
    pub progress_at_ms: Option<u64>,
}

/// The evidence behind a verdict, layer by layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    pub identity: Liveness,
    pub proc_state: Option<char>,
    pub turn: Turn,
    pub status_age_ms: Option<u64>,
    pub transcript: Option<FileMeta>,
    pub task_output: Option<FileMeta>,
    pub cpu_ms: Option<u64>,
    /// Whether tree CPU grew since the previous sample; `None` with no previous sample or no reading.
    pub cpu_grew: Option<bool>,
    pub children: Option<usize>,
    /// How long since the last sign of activity while busy.
    pub quiet_for_ms: Option<u64>,
    /// Names of the layers that could not be read.
    pub unknown: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub verdict: Verdict,
    pub evidence: Evidence,
    pub sample: Sample,
}

pub fn probe(
    rec: &Record,
    procs: &dyn ProcessTable,
    domain: &str,
    host: &dyn Host,
    prev: Option<&Sample>,
    t: &Thresholds,
) -> Probe {
    let now = host.now_ms();
    let identity = liveness::check(rec, procs, domain);
    let mut ev = Evidence::empty(identity);
    match identity {
        Liveness::Live => {}
        // another pid space: its pid is not ours to look up, and "cannot check" is never "dead"
        Liveness::OtherDomain => {
            ev.unknown.push("identity");
            return finish(Verdict::Unknown, ev, now, None);
        }
        _ => return finish(Verdict::Gone, ev, now, None),
    }
    let tree = host.procs().and_then(|rows| tree_of(&rows, rec.pid));
    ev.proc_state = tree.map(|x| x.state);
    ev.cpu_ms = tree.map(|x| x.cpu_ms);
    ev.children = tree.map(|x| x.live_children);
    ev.turn = host.turn(rec.pid);
    ev.transcript = host.transcript(rec).map(|m| clamp(m, now));
    ev.task_output = host.task_output(rec).map(|m| clamp(m, now));
    let progress = Progress::new(&ev, prev, now, t);
    ev.cpu_grew = progress.cpu_grew;
    ev.status_age_ms = ev.turn.updated_ms.filter(|u| *u <= now).map(|u| now - u);
    let last = [progress.at_ms, ev.turn.updated_ms.filter(|u| *u <= now)]
        .into_iter()
        .flatten()
        .max();
    ev.quiet_for_ms = last.map(|l| now - l);
    ev.unknown = unknown_layers(&ev);
    let verdict = decide(&ev, t);
    finish(verdict, ev, now, progress.at_ms)
}

fn finish(verdict: Verdict, ev: Evidence, now: u64, progress_at_ms: Option<u64>) -> Probe {
    let sample = Sample {
        at_ms: now,
        cpu_ms: ev.cpu_ms,
        transcript_len: ev.transcript.map(|m| m.len),
        progress_at_ms,
    };
    Probe {
        verdict,
        evidence: ev,
        sample,
    }
}

impl Evidence {
    fn empty(identity: Liveness) -> Evidence {
        Evidence {
            identity,
            proc_state: None,
            turn: Turn::default(),
            status_age_ms: None,
            transcript: None,
            task_output: None,
            cpu_ms: None,
            cpu_grew: None,
            children: None,
            quiet_for_ms: None,
            unknown: Vec::new(),
        }
    }
}

/// A file time ahead of the clock (skew) counts as now, never as the future.
fn clamp(m: FileMeta, now: u64) -> FileMeta {
    FileMeta {
        modified_ms: m.modified_ms.min(now),
        ..m
    }
}

#[derive(Clone, Copy)]
struct Tree {
    state: char,
    cpu_ms: u64,
    live_children: usize,
}

/// The session process, the CPU of it and everything below it, and how many descendants are alive. `None` when
/// the table has no row for the pid (it ended between the identity check and the read).
fn tree_of(rows: &[ProcRow], pid: u32) -> Option<Tree> {
    let root = rows.iter().find(|r| r.pid == pid)?;
    let mut seen = vec![pid];
    let mut cpu_ms = root.cpu_ms;
    let mut live_children = 0;
    let mut i = 0;
    while i < seen.len() {
        let parent = seen[i];
        i += 1;
        for r in rows.iter().filter(|r| r.ppid == parent) {
            if seen.contains(&r.pid) {
                continue;
            }
            seen.push(r.pid);
            cpu_ms += r.cpu_ms;
            live_children += usize::from(r.state != 'Z');
        }
    }
    Some(Tree {
        state: root.state,
        cpu_ms,
        live_children,
    })
}

struct Progress {
    cpu_grew: Option<bool>,
    /// The latest moment the session showed progress, from files and from the comparison with the last sample.
    at_ms: Option<u64>,
}

impl Progress {
    fn new(ev: &Evidence, prev: Option<&Sample>, now: u64, t: &Thresholds) -> Progress {
        let cpu_grew = match (prev.and_then(|p| p.cpu_ms), ev.cpu_ms) {
            (Some(before), Some(after)) => Some(after >= before.saturating_add(t.cpu_min_ms)),
            _ => None,
        };
        let len_grew = match (prev.and_then(|p| p.transcript_len), ev.transcript) {
            (Some(before), Some(m)) => m.len > before,
            _ => false,
        };
        let seen_now = (cpu_grew == Some(true) || len_grew).then_some(now);
        let at_ms = [
            ev.transcript.map(|m| m.modified_ms),
            ev.task_output.map(|m| m.modified_ms),
            prev.and_then(|p| p.progress_at_ms),
            seen_now,
        ]
        .into_iter()
        .flatten()
        .max();
        Progress { cpu_grew, at_ms }
    }
}

fn unknown_layers(ev: &Evidence) -> Vec<&'static str> {
    let layers = [
        ("process-state", ev.proc_state.is_none()),
        ("cpu", ev.cpu_ms.is_none()),
        ("children", ev.children.is_none()),
        ("turn", ev.turn.status.is_none()),
        ("transcript", ev.transcript.is_none()),
        ("task-output", ev.task_output.is_none()),
    ];
    layers.into_iter().filter(|l| l.1).map(|l| l.0).collect()
}

fn decide(ev: &Evidence, t: &Thresholds) -> Verdict {
    if matches!(ev.proc_state, Some('Z' | 'T')) {
        return Verdict::Stuck;
    }
    match ev.turn.status {
        None => Verdict::Unknown,
        Some(Status::Idle) => Verdict::WaitingOnHuman,
        Some(Status::Busy) => decide_busy(ev, t),
    }
}

fn decide_busy(ev: &Evidence, t: &Thresholds) -> Verdict {
    let Some(quiet) = ev.quiet_for_ms else {
        return Verdict::Unknown;
    };
    if quiet <= t.window_ms {
        return Verdict::Working;
    }
    if ev.children.is_some_and(|c| c > 0) {
        return Verdict::WaitingOnTool;
    }
    // stuck needs an eye on something: with no progress layer readable, a long quiet is not evidence
    let watchable = ev.transcript.is_some() || ev.cpu_ms.is_some() || ev.children.is_some();
    if quiet >= t.stuck_after_ms {
        return if watchable {
            Verdict::Stuck
        } else {
            Verdict::Unknown
        };
    }
    Verdict::Working
}

/// The registry's `status` and `statusUpdatedAt` from a record's text. Missing or surprising is `None`.
pub fn parse_turn(json: &str) -> Turn {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Turn::default();
    };
    let status = match v.get("status").and_then(|s| s.as_str()) {
        Some("busy") => Some(Status::Busy),
        Some("idle") => Some(Status::Idle),
        _ => None,
    };
    let updated_ms = v
        .get("statusUpdatedAt")
        .and_then(|u| u.as_u64())
        .filter(|u| *u > 0);
    Turn { status, updated_ms }
}

/// `ps -o time=` to milliseconds: `M:SS.cc`, `H:MM:SS` or `D-HH:MM:SS`. `None` for anything else.
pub fn parse_cpu_time(text: &str) -> Option<u64> {
    let (days, clock) = match text.split_once('-') {
        Some((d, rest)) => (d.parse::<u64>().ok()?, rest),
        None => (0, text),
    };
    let parts: Vec<&str> = clock.split(':').collect();
    let (hours, minutes, seconds) = match parts[..] {
        [m, s] => (0, m.parse::<u64>().ok()?, s),
        [h, m, s] => (h.parse::<u64>().ok()?, m.parse::<u64>().ok()?, s),
        _ => return None,
    };
    if seconds.is_empty() || !seconds.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return None;
    }
    let ms = (seconds.parse::<f64>().ok()? * 1000.0).round() as u64;
    Some(((days * 24 + hours) * 60 + minutes) * 60_000 + ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::liveness::fake::FakeTable;
    use std::cell::RefCell;

    const T0: &str = "Fri Nov 6 03:04:05 2026";
    const NOW: u64 = 10_000_000;
    const MIN: u64 = 60_000;

    fn rec() -> Record {
        Record {
            pid: 100,
            session_id: "s1".into(),
            cwd: "/w".into(),
            name: None,
            proc_start: T0.into(),
            pid_domain: "darwin".into(),
        }
    }

    fn table() -> FakeTable {
        FakeTable::default().with((100, 1), T0, "claude")
    }

    struct FakeHost {
        now: u64,
        rows: Option<Vec<ProcRow>>,
        turn: Turn,
        transcript: Option<FileMeta>,
        output: Option<FileMeta>,
        reads: RefCell<Vec<&'static str>>,
    }

    fn row(pid: u32, ppid: u32, state: char, cpu_ms: u64) -> ProcRow {
        ProcRow {
            pid,
            ppid,
            state,
            cpu_ms,
        }
    }

    fn host() -> FakeHost {
        FakeHost {
            now: NOW,
            rows: Some(vec![row(100, 1, 'S', 5_000)]),
            turn: Turn {
                status: Some(Status::Busy),
                updated_ms: Some(NOW - 30 * MIN),
            },
            transcript: Some(FileMeta {
                len: 1000,
                modified_ms: NOW - 30 * MIN,
            }),
            output: None,
            reads: RefCell::default(),
        }
    }

    impl Host for FakeHost {
        fn now_ms(&self) -> u64 {
            self.now
        }
        fn procs(&self) -> Option<Vec<ProcRow>> {
            self.reads.borrow_mut().push("procs");
            self.rows.clone()
        }
        fn turn(&self, _: u32) -> Turn {
            self.turn
        }
        fn transcript(&self, _: &Record) -> Option<FileMeta> {
            self.transcript
        }
        fn task_output(&self, _: &Record) -> Option<FileMeta> {
            self.output
        }
    }

    fn run(h: &FakeHost, prev: Option<&Sample>) -> Probe {
        probe(&rec(), &table(), "darwin", h, prev, &Thresholds::default())
    }

    fn verdict(h: &FakeHost) -> Verdict {
        run(h, None).verdict
    }

    #[test]
    fn a_missing_process_is_gone() {
        let p = probe(
            &rec(),
            &FakeTable::default(),
            "darwin",
            &host(),
            None,
            &Thresholds::default(),
        );
        assert_eq!(p.verdict, Verdict::Gone);
        assert_eq!(p.evidence.identity, Liveness::Dead);
    }

    #[test]
    fn a_reused_pid_is_gone() {
        let t = FakeTable::default().with((100, 1), "Sat Nov 7 00:00:00 2026", "claude");
        let p = probe(&rec(), &t, "darwin", &host(), None, &Thresholds::default());
        assert_eq!(p.verdict, Verdict::Gone);
    }

    #[test]
    fn another_machine_cannot_be_checked_and_is_unknown_not_gone() {
        let p = probe(
            &rec(),
            &table(),
            "linux",
            &host(),
            None,
            &Thresholds::default(),
        );
        assert_eq!(p.verdict, Verdict::Unknown);
        assert!(p.evidence.unknown.contains(&"identity"));
    }

    #[test]
    fn a_zombie_is_stuck() {
        let mut h = host();
        h.rows = Some(vec![row(100, 1, 'Z', 5_000)]);
        assert_eq!(verdict(&h), Verdict::Stuck);
    }

    #[test]
    fn a_stopped_process_is_stuck() {
        let mut h = host();
        h.rows = Some(vec![row(100, 1, 'T', 5_000)]);
        assert_eq!(verdict(&h), Verdict::Stuck);
    }

    #[test]
    fn idle_is_waiting_on_a_human_however_old() {
        let mut h = host();
        h.turn.status = Some(Status::Idle);
        assert_eq!(verdict(&h), Verdict::WaitingOnHuman);
    }

    #[test]
    fn busy_with_a_fresh_transcript_is_working() {
        let mut h = host();
        h.transcript = Some(FileMeta {
            len: 2000,
            modified_ms: NOW - 10_000,
        });
        assert_eq!(verdict(&h), Verdict::Working);
    }

    #[test]
    fn a_turn_that_only_just_began_is_working() {
        let mut h = host();
        h.turn.updated_ms = Some(NOW - 5_000);
        assert_eq!(verdict(&h), Verdict::Working);
    }

    #[test]
    fn fresh_background_output_is_working() {
        let mut h = host();
        h.output = Some(FileMeta {
            len: 5,
            modified_ms: NOW - 20_000,
        });
        assert_eq!(verdict(&h), Verdict::Working);
    }

    #[test]
    fn cpu_growth_between_two_samples_is_working() {
        let mut h = host();
        let first = run(&h, None);
        h.now = NOW + 5 * MIN;
        h.rows = Some(vec![row(100, 1, 'S', 9_000)]);
        let second = run(&h, Some(&first.sample));
        assert_eq!(second.verdict, Verdict::Working);
        assert_eq!(second.evidence.cpu_grew, Some(true));
    }

    #[test]
    fn cpu_growth_counts_the_whole_process_tree() {
        let mut h = host();
        h.rows = Some(vec![row(100, 1, 'S', 5_000), row(200, 100, 'R', 1_000)]);
        let first = run(&h, None);
        h.now = NOW + 5 * MIN;
        h.rows = Some(vec![row(100, 1, 'S', 5_000), row(200, 100, 'R', 9_000)]);
        let second = run(&h, Some(&first.sample));
        assert_eq!(second.evidence.cpu_ms, Some(14_000));
        assert_eq!(second.verdict, Verdict::Working);
    }

    #[test]
    fn growth_of_the_transcript_between_samples_is_working() {
        let mut h = host();
        let first = run(&h, None);
        h.now = NOW + 5 * MIN;
        h.transcript = Some(FileMeta {
            len: 1500,
            modified_ms: NOW - 30 * MIN,
        });
        assert_eq!(run(&h, Some(&first.sample)).verdict, Verdict::Working);
    }

    #[test]
    fn cpu_noise_below_the_minimum_is_not_progress() {
        let mut h = host();
        h.rows = Some(vec![row(100, 1, 'S', 5_000), row(200, 100, 'S', 0)]);
        let first = run(&h, None);
        h.now = NOW + 12 * MIN;
        h.rows = Some(vec![row(100, 1, 'S', 5_050), row(200, 100, 'S', 0)]);
        let second = run(&h, Some(&first.sample));
        assert_eq!(second.evidence.cpu_grew, Some(false));
    }

    #[test]
    fn busy_and_quiet_with_a_live_child_is_waiting_on_a_tool() {
        let mut h = host();
        h.rows = Some(vec![row(100, 1, 'S', 5_000), row(200, 100, 'S', 0)]);
        let p = run(&h, None);
        assert_eq!(p.verdict, Verdict::WaitingOnTool);
        assert_eq!(p.evidence.children, Some(1));
    }

    #[test]
    fn a_zombie_child_is_not_a_live_child() {
        let mut h = host();
        h.rows = Some(vec![row(100, 1, 'S', 5_000), row(200, 100, 'Z', 0)]);
        assert_eq!(verdict(&h), Verdict::Stuck);
    }

    #[test]
    fn busy_and_quiet_for_long_with_no_children_is_stuck() {
        assert_eq!(verdict(&host()), Verdict::Stuck);
    }

    #[test]
    fn busy_and_quiet_for_a_short_while_is_still_working() {
        let mut h = host();
        h.turn.updated_ms = Some(NOW - 5 * MIN);
        h.transcript = Some(FileMeta {
            len: 1000,
            modified_ms: NOW - 5 * MIN,
        });
        assert_eq!(verdict(&h), Verdict::Working);
    }

    #[test]
    fn the_first_sample_records_state_for_the_second() {
        let h = host();
        let first = run(&h, None);
        assert_eq!(first.evidence.cpu_grew, None);
        assert_eq!(first.sample.cpu_ms, Some(5_000));
        assert_eq!(first.sample.transcript_len, Some(1000));
        assert_eq!(first.sample.at_ms, NOW);
    }

    #[test]
    fn progress_seen_once_is_remembered_in_the_sample() {
        let mut h = host();
        h.transcript = Some(FileMeta {
            len: 2000,
            modified_ms: NOW - 1_000,
        });
        let first = run(&h, None);
        assert_eq!(first.sample.progress_at_ms, Some(NOW - 1_000));
    }

    #[test]
    fn an_unreadable_process_table_never_yields_gone() {
        let mut h = host();
        h.rows = None;
        let p = run(&h, None);
        assert_ne!(p.verdict, Verdict::Gone);
        assert!(p.evidence.unknown.contains(&"process-state"));
        assert!(p.evidence.unknown.contains(&"children"));
        assert!(p.evidence.unknown.contains(&"cpu"));
    }

    #[test]
    fn nothing_readable_while_busy_and_old_is_unknown_not_stuck() {
        let mut h = host();
        h.rows = None;
        h.transcript = None;
        assert_eq!(verdict(&h), Verdict::Unknown);
    }

    #[test]
    fn an_unreadable_turn_is_unknown_and_never_gone() {
        let mut h = host();
        h.turn = Turn::default();
        let p = run(&h, None);
        assert_eq!(p.verdict, Verdict::Unknown);
        assert!(p.evidence.unknown.contains(&"turn"));
    }

    #[test]
    fn a_timestamp_in_the_future_is_an_unknown_age() {
        let mut h = host();
        h.turn.updated_ms = Some(NOW + 10 * MIN);
        h.transcript = None;
        let p = run(&h, None);
        assert_eq!(p.evidence.status_age_ms, None);
    }

    #[test]
    fn an_unreadable_transcript_is_unknown_and_other_layers_still_decide() {
        let mut h = host();
        h.transcript = None;
        h.rows = Some(vec![row(100, 1, 'S', 5_000), row(200, 100, 'S', 0)]);
        let p = run(&h, None);
        assert_eq!(p.verdict, Verdict::WaitingOnTool);
        assert!(p.evidence.unknown.contains(&"transcript"));
    }

    #[test]
    fn parse_turn_reads_status_and_time() {
        let t = parse_turn(r#"{"pid":1,"status":"busy","statusUpdatedAt":1794723067082}"#);
        assert_eq!(t.status, Some(Status::Busy));
        assert_eq!(t.updated_ms, Some(1794723067082));
        assert_eq!(
            parse_turn(r#"{"status":"idle"}"#).status,
            Some(Status::Idle)
        );
    }

    #[test]
    fn parse_turn_treats_surprises_as_unknown() {
        for text in [
            "not json",
            "{}",
            r#"{"status":"thinking"}"#,
            r#"{"status":7}"#,
            r#"{"status":"busy","statusUpdatedAt":"yesterday"}"#,
            r#"{"status":"busy","statusUpdatedAt":-5}"#,
            r#"{"status":"busy","statusUpdatedAt":0}"#,
        ] {
            let t = parse_turn(text);
            assert!(
                t.updated_ms.is_none() && (t.status.is_none() || text.contains("busy")),
                "{text}: {t:?}"
            );
        }
        assert_eq!(parse_turn(r#"{"status":"thinking"}"#).status, None);
    }

    #[test]
    fn parse_cpu_time_reads_the_ps_formats() {
        assert_eq!(parse_cpu_time("0:00.78"), Some(780));
        assert_eq!(parse_cpu_time("82:51.93"), Some(82 * 60_000 + 51_930));
        assert_eq!(parse_cpu_time("1:02:03"), Some(3_723_000));
        assert_eq!(
            parse_cpu_time("2-01:00:00"),
            Some(2 * 86_400_000 + 3_600_000)
        );
        for bad in ["", "abc", "1:2:3:4", "-", "1:xx"] {
            assert_eq!(parse_cpu_time(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_probe_reads_the_table_once() {
        let h = host();
        run(&h, None);
        assert_eq!(h.reads.borrow().len(), 1);
    }
}
