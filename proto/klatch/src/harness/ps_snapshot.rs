//! One read of the whole process table (#1569), through a `ps` found at a fixed absolute path (#1568).
//!
//! `ps` is never looked up through `PATH`: a `ps` earlier on `PATH` would answer every liveness and walk question. The
//! program is the first regular executable of `PS_CANDIDATES`; with none, there is no process table and every answer is
//! "unknown", never a fall back to `PATH`. The child gets a cleared environment (only `TZ=UTC` and `LC_ALL=C`), reads
//! at most `Limits::cap` bytes of output, and is killed when it has not finished within `Limits::timeout`; a run that
//! fails, times out, overflows or prints a row that does not parse is an unreadable table, never a partial one.

use crate::harness::process::{Lookup, PID_LIMIT, ProcInfo, ProcessTable, squash};
use std::collections::HashMap;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Where `ps` may be, in order: macOS has it in `/bin`, Linux in `/bin` or `/usr/bin`.
pub(crate) const PS_CANDIDATES: [&str; 2] = ["/bin/ps", "/usr/bin/ps"];

/// What one `ps` run may cost.
#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub timeout: Duration,
    /// The most bytes of output read; a run that prints more is unreadable.
    pub cap: usize,
}

/// A real table is well under a megabyte (a few hundred bytes per process, thousands of processes); `ps` answers in
/// milliseconds.
pub(crate) const LIMITS: Limits = Limits {
    timeout: Duration::from_secs(5),
    cap: 16 * 1024 * 1024,
};

/// How often a running `ps` is looked at while waiting for it.
const POLL: Duration = Duration::from_millis(5);

/// The first of `candidates` that is an absolute path to a regular file anyone may execute. `None` for none: a bare name
/// (which would be looked up through `PATH`) is never accepted.
pub(crate) fn fixed_ps_in(candidates: &[&str]) -> Option<PathBuf> {
    candidates
        .iter()
        .map(Path::new)
        .filter(|p| p.is_absolute())
        .find(|p| {
            std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
        .map(Path::to_path_buf)
}

/// Run `program` with `pre` and `args`, and return what it printed on standard output as text. `None` when it cannot
/// start, does not finish within the timeout (it is killed), prints more than the cap, exits unsuccessfully or prints
/// something that is not text. The environment is cleared: `TZ=UTC` (the registry records start times in UTC) and
/// `LC_ALL=C` (a fixed `lstart` format) are all the child sees.
pub(crate) fn run_ps(
    program: &Path,
    pre: &[String],
    args: &[&str],
    limits: Limits,
) -> Option<String> {
    let mut child = Command::new(program)
        .args(pre)
        .args(args)
        .env_clear()
        .env("TZ", "UTC")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let cap = limits.cap;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        // one byte past the cap tells an overflow from an exact fit; dropping the pipe then ends the child (SIGPIPE)
        let _ = stdout.take(cap as u64 + 1).read_to_end(&mut out);
        out
    });
    let deadline = Instant::now() + limits.timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let out = reader.join().ok()?;
    if !status?.success() || out.len() > cap {
        return None;
    }
    String::from_utf8(out).ok()
}

/// The table `ps -A -o pid=,ppid=,lstart=,comm=` printed: one process per line, the start time as five words
/// (`Fri Nov  6 03:04:05 2026`) and the program name the rest of the line (it may hold spaces), whitespace collapsed
/// as a read of each field alone would have it. A row with no program name is left out (that pid is missing). `None` when there is no row or any row does not parse.
pub(crate) fn parse_table(text: &str) -> Option<HashMap<u32, ProcInfo>> {
    let mut table = HashMap::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.len() < 7 {
            return None;
        }
        let (pid, ppid) = (words[0].parse().ok()?, words[1].parse().ok()?);
        let command = words[7..].join(" ");
        if command.is_empty() {
            // `ps -o comm=` for this pid printed nothing: the per-pid read called such a process missing
            continue;
        }
        let start_utc = squash(&words[2..7].join(" "));
        table.insert(
            pid,
            ProcInfo {
                ppid,
                start_utc,
                command,
            },
        );
    }
    (!table.is_empty()).then_some(table)
}

/// The process table as it was at one moment. `None` inside means it could not be read.
pub(crate) struct Snapshot(pub(crate) Option<HashMap<u32, ProcInfo>>);

impl ProcessTable for Snapshot {
    fn lookup(&self, pid: u32) -> Lookup {
        match &self.0 {
            None => Lookup::Unreadable,
            Some(table) => table
                .get(&pid)
                .cloned()
                .map_or(Lookup::Missing, Lookup::Found),
        }
    }
}

/// A `ps` stand-in named by `KLATCH_TEST_PS`, so an integration test can count or fake the calls (it cannot put a `ps` on
/// PATH any more, #1568). Only in a build with debug assertions: a release build has no such variable, and the value must
/// be an absolute path.
#[cfg(debug_assertions)]
fn test_program() -> Option<PathBuf> {
    std::env::var_os("KLATCH_TEST_PS")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

#[cfg(not(debug_assertions))]
fn test_program() -> Option<PathBuf> {
    None
}

/// The process table read by running `program` (`pre` first among its arguments, for tests).
pub(crate) struct PsAt {
    pub program: Option<PathBuf>,
    pub pre_args: Vec<String>,
    pub limits: Limits,
}

impl PsAt {
    /// The real thing: the fixed `ps`, or no process table when there is none.
    pub(crate) fn fixed() -> Self {
        PsAt {
            program: test_program().or_else(|| fixed_ps_in(&PS_CANDIDATES)),
            pre_args: Vec::new(),
            limits: LIMITS,
        }
    }

    /// One run of `ps`, parsed.
    pub(crate) fn snapshot(&self) -> Snapshot {
        let text = self.program.as_deref().and_then(|program| {
            let args = ["-A", "-o", "pid=,ppid=,lstart=,comm="];
            run_ps(program, &self.pre_args, &args, self.limits)
        });
        Snapshot(text.and_then(|t| parse_table(&t)))
    }
}

impl ProcessTable for PsAt {
    fn lookup(&self, pid: u32) -> Lookup {
        if pid > PID_LIMIT {
            return Lookup::Missing;
        }
        self.snapshot().lookup(pid)
    }

    fn pin(&self) -> Option<Box<dyn ProcessTable>> {
        Some(Box::new(self.snapshot()))
    }
}

#[cfg(test)]
mod tests;
