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
