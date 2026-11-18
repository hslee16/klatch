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

    /// `None` when there is no such process and when the table cannot be read; a caller that must tell the two
    /// apart uses `lookup`.
    fn info(&self, pid: u32) -> Option<ProcInfo> {
        match self.lookup(pid) {
            Lookup::Found(info) => Some(info),
            Lookup::Missing | Lookup::Unreadable => None,
        }
    }
}

/// Collapse runs of whitespace, so `Nov  6` (padded day) and `Nov 6` compare equal.
pub(crate) fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The real process table, read with `ps` (macOS and Linux). The start time is read with `TZ=UTC`, because the
/// registry records it in UTC and a local-time comparison calls live sessions stale.
pub struct PsTable;

/// What one run of `ps` came to.
pub(crate) enum PsRun {
    /// The program could not be started.
    NotStarted,
    Ran {
        ok: bool,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
}

/// One `ps -o FIELD -p PID` read.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Field {
    Value(String),
    /// `ps` ran and said nothing: it prints nothing and exits 1 for a pid that does not exist.
    Missing,
    Unreadable,
}

/// A complaint on standard error, or output that is not text, means `ps` did not do its job; a silent failure is
/// how it says the pid is not there.
pub(crate) fn classify(run: PsRun) -> Field {
    let PsRun::Ran { ok, stdout, stderr } = run else {
        return Field::Unreadable;
    };
    let complained = !stderr.iter().all(u8::is_ascii_whitespace);
    let Ok(text) = String::from_utf8(stdout) else {
        return Field::Unreadable;
    };
    let text = squash(&text);
    match (text.is_empty(), ok, complained) {
        (false, true, _) => Field::Value(text),
        (true, _, false) => Field::Missing,
        _ => Field::Unreadable,
    }
}

fn ps(program: &str, pid: u32, field: &str) -> Field {
    let run = Command::new(program)
        .env("TZ", "UTC")
        .args(["-o", field, "-p", &pid.to_string()])
        .output()
        .map_or(PsRun::NotStarted, |out| PsRun::Ran {
            ok: out.status.success(),
            stdout: out.stdout,
            stderr: out.stderr,
        });
    classify(run)
}

/// The process table as `program` (normally `ps`) reads it. A pid that vanishes between the fields is missing.
pub(crate) fn lookup_via(program: &str, pid: u32) -> Lookup {
    let ppid = match ps(program, pid, "ppid=") {
        Field::Value(v) => v,
        Field::Missing => return Lookup::Missing,
        Field::Unreadable => return Lookup::Unreadable,
    };
    let Ok(ppid) = ppid.parse() else {
        return Lookup::Unreadable;
    };
    let mut rest = [ps(program, pid, "lstart="), ps(program, pid, "comm=")].into_iter();
    match (rest.next(), rest.next()) {
        (Some(Field::Value(start_utc)), Some(Field::Value(command))) => Lookup::Found(ProcInfo {
            ppid,
            start_utc,
            command,
        }),
        (Some(Field::Unreadable), _) | (_, Some(Field::Unreadable)) => Lookup::Unreadable,
        _ => Lookup::Missing,
    }
}

impl ProcessTable for PsTable {
    fn lookup(&self, pid: u32) -> Lookup {
        lookup_via("ps", pid)
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

    fn ran(ok: bool, stdout: &str, stderr: &str) -> PsRun {
        PsRun::Ran {
            ok,
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn a_ps_that_cannot_start_is_unreadable_not_missing() {
        assert_eq!(classify(PsRun::NotStarted), Field::Unreadable);
    }

    #[test]
    fn a_silent_failure_is_how_ps_says_there_is_no_such_pid() {
        assert_eq!(classify(ran(false, "", "")), Field::Missing);
        assert_eq!(classify(ran(false, "  \n", "")), Field::Missing);
        assert_eq!(classify(ran(true, "", "")), Field::Missing);
    }

    #[test]
    fn a_failure_with_a_complaint_is_unreadable() {
        let denied = ran(false, "", "ps: Operation not permitted\n");
        assert_eq!(classify(denied), Field::Unreadable);
        assert_eq!(
            classify(ran(false, "", "ps: error\n")),
            Field::Unreadable,
            "a complaint decides, whatever the exit code"
        );
    }

    #[test]
    fn output_that_is_not_text_is_unreadable() {
        let run = PsRun::Ran {
            ok: true,
            stdout: vec![0xff, 0xfe],
            stderr: vec![],
        };
        assert_eq!(classify(run), Field::Unreadable);
    }

    #[test]
    fn a_value_is_squashed() {
        assert_eq!(
            classify(ran(true, " Fri Nov  6 03:04:05 2026\n", "")),
            Field::Value("Fri Nov 6 03:04:05 2026".into())
        );
    }

    #[test]
    fn a_successful_run_with_a_warning_still_has_its_value() {
        assert_eq!(
            classify(ran(true, "42\n", "warning: x\n")),
            Field::Value("42".into())
        );
    }

    #[test]
    fn a_program_that_does_not_exist_makes_the_table_unreadable() {
        assert_eq!(
            lookup_via("/nonexistent/klatch-no-such-ps", 1234),
            Lookup::Unreadable
        );
    }

    #[test]
    fn a_ps_that_exits_silently_with_failure_reads_as_a_missing_pid() {
        let program = ["/usr/bin/false", "/bin/false"]
            .into_iter()
            .find(|p| std::path::Path::new(p).exists())
            .expect("a false program");
        assert_eq!(lookup_via(program, 1234), Lookup::Missing);
    }

    #[test]
    fn a_ps_that_complains_makes_the_table_unreadable() {
        // `ls` given ps's arguments names a file that is not there: a complaint on stderr and a failure exit
        let program = ["/bin/ls", "/usr/bin/ls"]
            .into_iter()
            .find(|p| std::path::Path::new(p).exists())
            .expect("an ls program");
        assert_eq!(lookup_via(program, 1234), Lookup::Unreadable);
    }

    #[test]
    fn the_real_table_finds_this_process_and_misses_a_pid_that_is_not_there() {
        assert!(matches!(
            PsTable.lookup(std::process::id()),
            Lookup::Found(_)
        ));
        // a process that has ended and been waited for is not there, and ps says so silently
        let mut child = Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert_eq!(PsTable.lookup(pid), Lookup::Missing);
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
