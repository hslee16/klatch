//! Is a registry record about a session that is running now? Record age is not a test: idle sessions go
//! hours without updating. A record is live only if its pid exists, was started when the record says, and is
//! the harness (RESEARCH-NOTES section 17).

use crate::process::ProcessTable;
use crate::record::Record;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    Live,
    /// The record belongs to another machine or pid space, so it cannot be checked here.
    OtherDomain,
    /// No process has that pid.
    Dead,
    /// The pid exists but was started at another time: the pid was reused after the session ended.
    Reused,
    /// The pid is a live process started at the recorded time, but it is not the harness.
    NotHarness,
}

pub fn check(rec: &Record, procs: &dyn ProcessTable, domain: &str) -> Liveness {
    if rec.pid_domain != domain {
        return Liveness::OtherDomain;
    }
    let Some(info) = procs.info(rec.pid) else {
        return Liveness::Dead;
    };
    if info.start_utc != rec.proc_start {
        return Liveness::Reused;
    }
    if !is_claude_code(&info.command) {
        return Liveness::NotHarness;
    }
    Liveness::Live
}

/// True for the Claude Code program name.
pub fn is_claude_code(command: &str) -> bool {
    command.rsplit('/').next() == Some("claude")
}

#[cfg(test)]
pub(crate) mod fake {
    use crate::process::{ProcInfo, ProcessTable};
    use std::collections::HashMap;

    /// A made-up process table.
    #[derive(Default)]
    pub struct FakeTable(pub HashMap<u32, ProcInfo>);

    impl FakeTable {
        pub fn with(mut self, (pid, ppid): (u32, u32), start: &str, command: &str) -> Self {
            self.0.insert(
                pid,
                ProcInfo {
                    ppid,
                    start_utc: start.into(),
                    command: command.into(),
                },
            );
            self
        }
    }

    impl ProcessTable for FakeTable {
        fn info(&self, pid: u32) -> Option<ProcInfo> {
            self.0.get(&pid).cloned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeTable;
    use super::*;

    fn rec(pid: u32) -> Record {
        Record {
            pid,
            session_id: "s1".into(),
            cwd: "/w".into(),
            name: None,
            proc_start: "Fri Nov 6 03:04:05 2026".into(),
            pid_domain: "darwin".into(),
        }
    }

    const T: &str = "Fri Nov 6 03:04:05 2026";

    #[test]
    fn live_when_alive_started_then_and_is_the_harness() {
        let t = FakeTable::default().with((10, 1), T, "claude");
        assert_eq!(check(&rec(10), &t, "darwin"), Liveness::Live);
    }

    #[test]
    fn a_path_to_the_program_still_counts() {
        let t = FakeTable::default().with((10, 1), T, "/usr/local/bin/claude");
        assert_eq!(check(&rec(10), &t, "darwin"), Liveness::Live);
    }

    #[test]
    fn dead_pid() {
        assert_eq!(
            check(&rec(10), &FakeTable::default(), "darwin"),
            Liveness::Dead
        );
    }

    #[test]
    fn reused_pid_has_a_different_start_time() {
        let t = FakeTable::default().with((10, 1), "Sat Nov 7 09:00:00 2026", "claude");
        assert_eq!(check(&rec(10), &t, "darwin"), Liveness::Reused);
    }

    #[test]
    fn same_pid_and_start_but_a_different_program() {
        let t = FakeTable::default().with((10, 1), T, "vim");
        assert_eq!(check(&rec(10), &t, "darwin"), Liveness::NotHarness);
    }

    #[test]
    fn another_pid_domain_cannot_be_checked() {
        let t = FakeTable::default().with((10, 1), T, "claude");
        assert_eq!(check(&rec(10), &t, "linux"), Liveness::OtherDomain);
    }

    #[test]
    fn the_domain_is_checked_before_the_pid_so_a_foreign_pid_is_never_looked_up() {
        // pid 10 is dead here, but the record is from another domain, so it is not called dead
        assert_eq!(
            check(&rec(10), &FakeTable::default(), "linux"),
            Liveness::OtherDomain
        );
    }

    #[test]
    fn program_name_must_be_exactly_claude() {
        assert!(is_claude_code("claude"));
        assert!(is_claude_code("/opt/x/claude"));
        assert!(!is_claude_code("claude-helper"));
        assert!(!is_claude_code("notclaude"));
        assert!(!is_claude_code(""));
    }
}
