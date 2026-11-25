//! The process table, behind a trait so tests can use a made-up one.

use crate::harness::ps_snapshot::PsAt;

/// What is known about one process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    pub ppid: u32,
    /// Start time as `ps -o lstart=` prints it in UTC, whitespace collapsed (for example `Fri Nov 6 03:04:05 2026`).
    pub start_utc: String,
    /// The program name, as `ps -o comm=` prints it.
    pub command: String,
}

/// What a process table says about one pid. Three answers, because "no such process" and "I could not look" are
/// different facts (#525): the first lets a caller call a session gone, the second must never.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    Found(ProcInfo),
    /// The table was read and has no process with that pid.
    Missing,
    /// The table could not be read (`ps` cannot run, was refused, or printed something unreadable).
    Unreadable,
}

pub trait ProcessTable {
    fn lookup(&self, pid: u32) -> Lookup;

    /// A copy of the table as it is now, for a walk that looks at many pids: one read instead of one per pid (#1569).
    /// `None` (the default) for a table that is already cheap to ask, such as a made-up one.
    fn pin(&self) -> Option<Box<dyn ProcessTable>> {
        None
    }

    /// `None` when there is no such process and when the table cannot be read; a caller that must tell the two
    /// apart uses `lookup`.
    fn info(&self, pid: u32) -> Option<ProcInfo> {
        match self.lookup(pid) {
            Lookup::Found(info) => Some(info),
            Lookup::Missing | Lookup::Unreadable => None,
        }
    }
}

/// Run `f` on a pinned snapshot of `procs`, or on `procs` itself when it cannot pin one.
pub(crate) fn pinned<R>(procs: &dyn ProcessTable, f: impl FnOnce(&dyn ProcessTable) -> R) -> R {
    match procs.pin() {
        Some(snapshot) => f(&*snapshot),
        None => f(procs),
    }
}

/// Collapse runs of whitespace, so `Nov  6` (padded day) and `Nov 6` compare equal.
pub(crate) fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The real process table, read with the `ps` at a fixed path (never found through `PATH`, #1568). Each lookup is one
/// `ps` run over the whole table, and a walk pins one snapshot with `pin` instead (#1569). The start time is read with
/// `TZ=UTC`, because the registry records it in UTC and a local-time comparison calls live sessions stale.
pub struct PsTable;

/// The largest pid this kind of system can number. A pid above it is no process, and is answered without running `ps`.
#[cfg(target_os = "macos")]
pub(crate) const PID_LIMIT: u32 = 99_999;
#[cfg(not(target_os = "macos"))]
pub(crate) const PID_LIMIT: u32 = 4_194_304;

impl ProcessTable for PsTable {
    fn lookup(&self, pid: u32) -> Lookup {
        PsAt::fixed().lookup(pid)
    }

    fn pin(&self) -> Option<Box<dyn ProcessTable>> {
        PsAt::fixed().pin()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn squash_collapses_the_padded_day() {
        assert_eq!(
            squash("Fri Nov  6 03:04:05 2026\n"),
            "Fri Nov 6 03:04:05 2026"
        );
    }

    #[test]
    fn ps_table_describes_this_process() {
        let info = PsTable.info(std::process::id()).expect("own pid exists");
        assert_eq!(info.ppid, std::os::unix::process::parent_id());
        assert!(!info.command.is_empty());
        let year = info.start_utc.rsplit(' ').next().unwrap();
        assert_eq!(
            year.len(),
            4,
            "lstart ends with the year: {}",
            info.start_utc
        );
    }

    #[test]
    fn ps_table_says_nothing_about_a_missing_pid() {
        assert_eq!(PsTable.info(u32::MAX - 1), None);
    }

    #[test]
    fn the_real_table_finds_this_process_and_misses_a_pid_that_is_not_there() {
        assert!(matches!(
            PsTable.lookup(std::process::id()),
            Lookup::Found(_)
        ));
        // a process that has ended and been waited for is not there, and ps says so silently
        let mut child = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert_eq!(PsTable.lookup(pid), Lookup::Missing);
        // a pid no system can number is not a process, and ps is not asked
        assert_eq!(PsTable.lookup(u32::MAX - 1), Lookup::Missing);
        assert_eq!(PsTable.lookup(PID_LIMIT + 1), Lookup::Missing);
    }

    #[test]
    fn pinned_uses_the_snapshot_when_there_is_one_and_the_table_when_not() {
        struct Pins(std::cell::Cell<u32>);
        impl ProcessTable for Pins {
            fn lookup(&self, _: u32) -> Lookup {
                Lookup::Unreadable
            }
            fn pin(&self) -> Option<Box<dyn ProcessTable>> {
                self.0.set(self.0.get() + 1);
                Some(Box::new(crate::harness::ps_snapshot::Snapshot(None)))
            }
        }
        let table = Pins(std::cell::Cell::new(0));
        assert_eq!(pinned(&table, |t| t.lookup(1)), Lookup::Unreadable);
        assert_eq!(table.0.get(), 1);
        struct Plain;
        impl ProcessTable for Plain {
            fn lookup(&self, _: u32) -> Lookup {
                Lookup::Missing
            }
        }
        assert_eq!(pinned(&Plain, |t| t.lookup(1)), Lookup::Missing);
    }

    #[test]
    fn info_is_some_only_when_found() {
        struct One(Lookup);
        impl ProcessTable for One {
            fn lookup(&self, _: u32) -> Lookup {
                self.0.clone()
            }
        }
        assert_eq!(One(Lookup::Missing).info(1), None);
        assert_eq!(One(Lookup::Unreadable).info(1), None);
    }
}
