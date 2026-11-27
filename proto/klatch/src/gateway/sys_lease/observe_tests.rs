//! `observe_in` against a made-up process table and host (#1706): a same-user process can write a registry record
//! carrying any session id under any pid, so the records of one session id must not be trusted one by one.

use super::*;
use crate::harness::{Lookup, ProcInfo, Status};
use std::collections::HashMap;

const NOW: u64 = 10_000_000;
const START: &str = "Fri Nov 27 01:00:00 2026";

#[derive(Default)]
struct Table(HashMap<u32, ProcInfo>);

impl Table {
    fn with(mut self, pid: u32, start: &str, command: &str) -> Table {
        self.0.insert(
            pid,
            ProcInfo {
                ppid: 1,
                start_utc: start.into(),
                command: command.into(),
            },
        );
        self
    }
}

impl ProcessTable for Table {
    fn lookup(&self, pid: u32) -> Lookup {
        self.0
            .get(&pid)
            .cloned()
            .map_or(Lookup::Missing, Lookup::Found)
    }
}

struct Fake(Vec<u32>);

impl Host for Fake {
    fn now_ms(&self) -> u64 {
        NOW
    }
    fn procs(&self) -> Option<Vec<ProcRow>> {
        Some(
            self.0
                .iter()
                .map(|pid| ProcRow {
                    pid: *pid,
                    ppid: 1,
                    state: 'S',
                    cpu_ms: 10,
                    elapsed_ms: None,
                })
                .collect(),
        )
    }
    fn turn(&self, _: u32) -> Turn {
        Turn {
            status: Some(Status::Idle),
            updated_ms: Some(NOW - 60_000),
        }
    }
    fn transcript(&self, _: &Record) -> Option<FileMeta> {
        None
    }
    fn task_output(&self, _: &Record) -> Option<FileMeta> {
        None
    }
}

fn dir(tag: &str) -> (crate::store::test_dir::TestDir, FsRegistry) {
    let d = crate::store::test_dir::TestDir::new(&format!("observe-{tag}"));
    let sessions = d.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    (d, FsRegistry { dir: sessions })
}

fn write(reg: &FsRegistry, pid: u32, session: &str, start: &str) {
    let json = format!(
        r#"{{"pid":{pid},"sessionId":"{session}","cwd":"/w","procStart":"{start}","pidDomain":"darwin","status":"idle","statusUpdatedAt":1}}"#
    );
    std::fs::write(reg.dir.join(format!("{pid}.json")), json).unwrap();
}

fn seen(reg: &FsRegistry, table: &Table, session: &str) -> Option<Observed> {
    let procs: Vec<u32> = table.0.keys().copied().collect();
    let host = Fake(procs.clone());
    let alive = Some(procs.into_iter().collect());
    observe_in(reg, table, &host, "darwin", session, &alive)
}

#[test]
fn one_record_of_a_live_harness_reads_as_waiting_on_the_human() {
    let (_d, reg) = dir("one");
    write(&reg, 5000, "victim", START);
    let table = Table::default().with(5000, START, "claude");
    assert_eq!(
        seen(&reg, &table, "victim").unwrap().verdict,
        Verdict::WaitingOnHuman
    );
}

#[test]
fn one_record_of_a_dead_process_reads_as_gone() {
    let (_d, reg) = dir("gone");
    write(&reg, 5000, "victim", START);
    assert_eq!(
        seen(&reg, &Table::default(), "victim").unwrap().verdict,
        Verdict::Gone
    );
}

#[test]
fn a_forged_record_under_a_low_live_pid_does_not_make_a_live_member_read_as_gone() {
    // #1706 (1c): pid 1 is always alive and is not the harness, so its record probes as Gone; taken first, it would
    // shorten the claims of a member that is running
    let (_d, reg) = dir("low");
    write(&reg, 5000, "victim", START);
    write(&reg, 1, "victim", START);
    let table =
        Table::default()
            .with(5000, START, "claude")
            .with(1, "Mon Jan  1 00:00:00 2026", "launchd");
    assert_eq!(seen(&reg, &table, "victim"), None);
}

#[test]
fn a_forged_record_for_a_dead_member_under_a_harness_named_process_does_not_keep_its_claims_alive()
{
    // #1706 (1d): the dead member's real record sits under a dead pid; a forged one under a live process named claude
    // would read as waiting on the human, and the lease would be extended on every sweep
    let (_d, reg) = dir("dead");
    write(&reg, 6000, "dead-member", START);
    write(&reg, 7000, "dead-member", START);
    let table = Table::default().with(7000, START, "claude");
    assert_eq!(seen(&reg, &table, "dead-member"), None);
}

#[test]
fn a_restarted_harness_with_the_stale_file_of_the_old_pid_is_left_to_its_ttl() {
    // the same shape as (1d) and indistinguishable from it, so neither extended nor shortened
    let (_d, reg) = dir("restart");
    write(&reg, 6000, "resumed", START);
    write(&reg, 7000, "resumed", START);
    let table = Table::default().with(7000, START, "claude");
    assert_eq!(seen(&reg, &table, "resumed"), None);
}

#[test]
fn two_records_that_are_both_gone_read_as_gone() {
    let (_d, reg) = dir("both-gone");
    write(&reg, 6000, "s", START);
    write(&reg, 7000, "s", START);
    assert_eq!(
        seen(&reg, &Table::default(), "s").unwrap().verdict,
        Verdict::Gone
    );
}

#[test]
fn records_of_other_sessions_are_not_a_conflict() {
    let (_d, reg) = dir("others");
    write(&reg, 5000, "mine", START);
    write(&reg, 1, "other", START);
    let table = Table::default()
        .with(5000, START, "claude")
        .with(1, "x", "launchd");
    assert_eq!(
        seen(&reg, &table, "mine").unwrap().verdict,
        Verdict::WaitingOnHuman
    );
}

/// Counts what is asked of the real table: `ps -A` runs for each lookup on it, but only once for a pinned snapshot.
struct Counting {
    table: Table,
    direct: std::cell::Cell<usize>,
    pins: std::cell::Cell<usize>,
}

struct Snapshot(HashMap<u32, ProcInfo>);

impl ProcessTable for Snapshot {
    fn lookup(&self, pid: u32) -> Lookup {
        self.0
            .get(&pid)
            .cloned()
            .map_or(Lookup::Missing, Lookup::Found)
    }
}

impl ProcessTable for Counting {
    fn lookup(&self, pid: u32) -> Lookup {
        self.direct.set(self.direct.get() + 1);
        self.table.lookup(pid)
    }
    fn pin(&self) -> Option<Box<dyn ProcessTable>> {
        self.pins.set(self.pins.get() + 1);
        Some(Box::new(Snapshot(self.table.0.clone())))
    }
}

#[test]
fn many_records_of_one_session_cost_one_read_of_the_process_table() {
    // #1706 review: each lookup on the real table is a `ps -A`, so a flood of forged records must be judged against
    // one snapshot
    let (_d, reg) = dir("cost");
    let mut table = Table::default().with(5000, START, "claude");
    write(&reg, 5000, "victim", START);
    for pid in 1..=60u32 {
        write(&reg, pid, "victim", START);
        table = table.with(pid, "other", "launchd");
    }
    let counting = Counting {
        table,
        direct: Default::default(),
        pins: Default::default(),
    };
    let host = Fake(counting.table.0.keys().copied().collect());
    let alive = Some(counting.table.0.keys().copied().collect());
    assert_eq!(
        observe_in(&reg, &counting, &host, "darwin", "victim", &alive),
        None
    );
    assert_eq!(
        counting.direct.get(),
        0,
        "a record was judged on the live table"
    );
    assert_eq!(counting.pins.get(), 1);
}

/// A table of `n` live pids from 100 up (named like nothing the harness runs) plus the given rows.
fn flooded(n: u32, extra: Table) -> Table {
    (100..100 + n).fold(extra, |t, pid| t.with(pid, "other", "sleep"))
}

fn forge_fresh(reg: &FsRegistry, n: u32, session: &str) {
    for pid in 100..100 + n {
        write(reg, pid, session, "other");
    }
}

fn age(reg: &FsRegistry, pid: u32) {
    std::fs::File::options()
        .write(true)
        .open(reg.dir.join(format!("{pid}.json")))
        .unwrap()
        .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
        .unwrap();
}

#[test]
fn a_flood_of_fresher_live_pid_records_hiding_the_real_one_leaves_the_lease_alone() {
    // #1706 (1c) flood residual: the forged pid-1 record is inside the bound, the real older one is not; one
    // candidate would read as Gone and shorten the victim's claims
    let (_d, reg) = dir("flood");
    write(&reg, 5000, "victim", START);
    age(&reg, 5000);
    forge_fresh(&reg, 256, "decoy");
    // written last, so the freshest: inside the bound whatever the clock resolution
    write(&reg, 1, "victim", START);
    std::thread::sleep(std::time::Duration::from_millis(20));
    write(&reg, 1, "victim", START);
    let table = flooded(
        256,
        Table::default()
            .with(5000, START, "claude")
            .with(1, "Mon Jan  1 00:00:00 2026", "launchd"),
    );
    assert_eq!(seen(&reg, &table, "victim"), None);
}

#[test]
fn a_flood_that_pushes_the_session_out_of_the_listing_is_not_a_verdict_of_absence() {
    let (_d, reg) = dir("flood-absent");
    write(&reg, 5000, "victim", START);
    age(&reg, 5000);
    forge_fresh(&reg, 256, "decoy");
    let table = flooded(256, Table::default().with(5000, START, "claude"));
    assert_eq!(seen(&reg, &table, "victim"), None);
}

#[test]
fn a_real_record_below_the_bound_is_still_found() {
    // 255 forged plus the real one is 256: nothing is cut
    let (_d, reg) = dir("below");
    write(&reg, 5000, "victim", START);
    age(&reg, 5000);
    forge_fresh(&reg, 255, "decoy");
    let table = flooded(255, Table::default().with(5000, START, "claude"));
    assert_eq!(
        seen(&reg, &table, "victim").unwrap().verdict,
        Verdict::WaitingOnHuman
    );
}

/// The ticket #1712 scenario: the dead member's real record under dead pid 6000, 255 decoys of running pids, and a
/// forged record of the same session under pid 7000 (a live process named like the harness).
fn dead_member_flood(reg: &FsRegistry) -> Table {
    write(reg, 6000, "victim", START);
    age(reg, 6000);
    forge_fresh(reg, 255, "decoy");
    write(reg, 7000, "victim", START);
    flooded(255, Table::default().with(7000, START, "claude"))
}

#[test]
fn a_flood_of_running_pid_records_cannot_hide_the_dead_members_real_record_from_a_forgery() {
    // #1712: pid 6000 is dead, so the bound fills with running pids and 6000 is never read in full; its claim is
    // still a second (Gone) claim, so the forged alive record no longer reads as the only one
    let (_d, reg) = dir("dead-member-flood");
    let table = dead_member_flood(&reg);
    assert_eq!(seen(&reg, &table, "victim"), None);
}

#[test]
fn dead_pid_names_left_past_the_open_bound_turn_an_alive_answer_into_none_and_let_gone_stand() {
    // more dead-pid files than one pass opens: what is unread may hold the second claim of the session
    let (_d, reg) = dir("dead-unread");
    for pid in 100..100 + 4200u32 {
        write(&reg, pid, "old", "other");
    }
    write(&reg, 7000, "victim", START);
    let alive = Table::default().with(7000, START, "claude");
    assert_eq!(seen(&reg, &alive, "victim"), None, "alive answer");
    let dead = Table::default();
    assert_eq!(
        seen(&reg, &dead, "victim").unwrap().verdict,
        Verdict::Gone,
        "Gone stands"
    );
}

#[test]
fn many_leftover_dead_pid_files_with_one_real_record_change_nothing() {
    // 600 dead-pid files plus one real record: found, not truncated, behaviour as before
    let (_d, reg) = dir("dead-many");
    write(&reg, 7000, "victim", START);
    for pid in 100..700u32 {
        write(&reg, pid, "old", "other");
    }
    let table = Table::default().with(7000, START, "claude");
    assert_eq!(
        seen(&reg, &table, "victim").unwrap().verdict,
        Verdict::WaitingOnHuman
    );
}
