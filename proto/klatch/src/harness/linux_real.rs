//! Session discovery against a real Linux process table (#171). Compiled and run on Linux only (CI's ubuntu job,
//! or `proto/linux-discovery/run.sh` in a Debian container on any machine); on macOS the module is absent, so the
//! tests count as skipped there (the macOS equivalents are the `harness_join` integration tests).
//!
//! A fake harness is a symlink to `/bin/sh` named for the program under test, so the kernel's `comm` is the symlink
//! name, as it is for a real `claude`. It runs `sleep 300 &` (killed when the test drops its stdin) so there is a real descendant to start the walk from.
//! The registry is a fixture directory of `<pid>.json` records written here: the real `~/.claude/sessions` is never
//! read or written. What this does not observe is a record written by a real Claude Code (#797).

use crate::harness::ancestry::{Miss, find_session};
use crate::harness::liveness::Liveness;
use crate::harness::process::{ProcessTable, PsTable, squash};
use crate::harness::record::FsRegistry;
use crate::store::test_dir::TestDir;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// A scratch directory (removed on drop) with a registry directory in it.
struct Scratch(TestDir);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir = TestDir::new(tag);
        std::fs::create_dir_all(dir.join("registry")).unwrap();
        Scratch(dir)
    }
    fn registry(&self) -> FsRegistry {
        FsRegistry {
            dir: self.0.join("registry"),
        }
    }
}

/// A running fake harness and the pid of its child.
struct Fake {
    harness: Child,
    child: u32,
}

impl Drop for Fake {
    fn drop(&mut self) {
        // closing its stdin ends the shell's `read`, which kills the child and exits
        drop(self.harness.stdin.take());
        let _ = self.harness.wait();
    }
}

/// Start a program whose name (and so `comm`) is `name`, with one child.
fn spawn(dir: &Path, name: &str) -> Fake {
    let bin = dir.join(name);
    std::os::unix::fs::symlink("/bin/sh", &bin).unwrap();
    let mut harness = Command::new(&bin)
        .args(["-c", "sleep 300 & c=$!; echo $c; read x; kill -9 $c"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(harness.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let child = line.trim().parse().expect("the shell printed its child");
    Fake { harness, child }
}

/// The start time and command the real table gives for `pid`.
fn observe(pid: u32) -> (String, String) {
    let info = PsTable.info(pid).expect("the process is in the table");
    (info.start_utc, info.command)
}

fn write_record(s: &Scratch, pid: u32, start: &str, domain: &str) {
    let json = format!(
        r#"{{"pid":{pid},"sessionId":"real-{pid}","cwd":"/w","name":"lin","procStart":"{start}","pidDomain":"{domain}"}}"#
    );
    std::fs::write(s.0.join("registry").join(format!("{pid}.json")), json).unwrap();
}

#[test]
fn a_descendant_of_a_live_claude_finds_its_session() {
    let s = Scratch::new("live");
    let f = spawn(&s.0, "claude");
    let (start, command) = observe(f.harness.id());
    assert_eq!(command, "claude", "ps prints the symlink name as comm");
    write_record(&s, f.harness.id(), &start, "linux");
    let found = find_session(f.child, &PsTable, &s.registry(), "linux").expect("the session");
    assert_eq!(found.pid, f.harness.id());
    assert_eq!(found.session_id, format!("real-{}", f.harness.id()));
}

#[test]
fn a_padded_day_in_the_record_matches_the_real_lstart() {
    // procps pads a one digit day with a space; the record text keeps it and the comparison squashes it
    let s = Scratch::new("pad");
    let f = spawn(&s.0, "claude");
    let (start, _) = observe(f.harness.id());
    let day_padded = {
        let mut parts: Vec<&str> = start.split(' ').collect();
        if parts.get(2).is_some_and(|d| d.len() == 1) {
            parts[2] = " ";
        }
        parts.join(" ")
    };
    assert_eq!(squash(&day_padded), start);
    write_record(&s, f.harness.id(), &day_padded, "linux");
    assert!(find_session(f.child, &PsTable, &s.registry(), "linux").is_ok());
}

#[test]
fn an_unrelated_process_does_not_find_the_session() {
    let s = Scratch::new("unrelated");
    let f = spawn(&s.0, "claude");
    let (start, _) = observe(f.harness.id());
    write_record(&s, f.harness.id(), &start, "linux");
    // this test process is not below the fake harness
    assert_eq!(
        find_session(std::process::id(), &PsTable, &s.registry(), "linux"),
        Err(Miss::NoSession)
    );
}

#[test]
fn a_record_for_a_reused_pid_is_stale() {
    let s = Scratch::new("reused");
    let f = spawn(&s.0, "claude");
    write_record(&s, f.harness.id(), "Mon Jan 1 00:00:00 2001", "linux");
    assert_eq!(
        find_session(f.child, &PsTable, &s.registry(), "linux"),
        Err(Miss::Stale(Liveness::Reused))
    );
}

#[test]
fn a_retitled_background_session_is_still_the_harness() {
    let s = Scratch::new("bg");
    let f = spawn(&s.0, "claude bg-spare");
    let (start, command) = observe(f.harness.id());
    assert_eq!(command, "claude bg-spare");
    let comm = std::fs::read_to_string(format!("/proc/{}/comm", f.harness.id())).unwrap();
    assert_eq!(comm.trim_end(), "claude bg-spare", "15 characters, no cut");
    write_record(&s, f.harness.id(), &start, "linux");
    assert!(find_session(f.child, &PsTable, &s.registry(), "linux").is_ok());
}

#[test]
fn another_program_with_a_record_is_not_the_harness() {
    // `claude-code-sandbox` is cut to 15 characters by the kernel; `node` and `claude-helper` are other programs
    for name in ["claude-code-sandbox", "node", "claude-helper"] {
        let s = Scratch::new("other");
        let f = spawn(&s.0, name);
        let (start, command) = observe(f.harness.id());
        assert!(command.len() <= 15, "comm is cut: {command}");
        write_record(&s, f.harness.id(), &start, "linux");
        assert_eq!(
            find_session(f.child, &PsTable, &s.registry(), "linux"),
            Err(Miss::Stale(Liveness::NotHarness)),
            "{name}"
        );
    }
}

#[test]
fn a_record_from_another_pid_domain_is_never_live_here() {
    let s = Scratch::new("domain");
    let f = spawn(&s.0, "claude");
    let (start, _) = observe(f.harness.id());
    write_record(&s, f.harness.id(), &start, "darwin");
    assert_eq!(
        find_session(f.child, &PsTable, &s.registry(), "linux"),
        Err(Miss::Stale(Liveness::OtherDomain))
    );
}
