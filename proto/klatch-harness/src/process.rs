//! The process table, behind a trait so tests can use a made-up one.

use std::process::Command;

/// What is known about one process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    pub ppid: u32,
    /// Start time as `ps -o lstart=` prints it in UTC, whitespace collapsed (for example `Fri Nov 6 03:04:05 2026`).
    pub start_utc: String,
    /// The program name, as `ps -o comm=` prints it.
    pub command: String,
}

pub trait ProcessTable {
    /// `None` when no process with that pid exists.
    fn info(&self, pid: u32) -> Option<ProcInfo>;
}

/// Collapse runs of whitespace, so `Nov  6` (padded day) and `Nov 6` compare equal.
pub(crate) fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The real process table, read with `ps` (macOS and Linux). The start time is read with `TZ=UTC`, because the
/// registry records it in UTC and a local-time comparison calls live sessions stale.
pub struct PsTable;

fn ps(pid: u32, field: &str) -> Option<String> {
    let out = Command::new("ps")
        .env("TZ", "UTC")
        .args(["-o", field, "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let text = squash(&text);
    (!text.is_empty()).then_some(text)
}

impl ProcessTable for PsTable {
    fn info(&self, pid: u32) -> Option<ProcInfo> {
        let ppid = ps(pid, "ppid=")?.parse().ok()?;
        Some(ProcInfo {
            ppid,
            start_utc: ps(pid, "lstart=")?,
            command: ps(pid, "comm=")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
