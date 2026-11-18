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

fn ps(pid: u32, field: &str) -> Field {
    let run = Command::new("ps")
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

/// The process table as `read` (one `ps -o FIELD -p PID` read) gives it. A pid that vanishes between the three
/// fields is missing.
pub(crate) fn lookup_with(read: &dyn Fn(&str) -> Field) -> Lookup {
    let ppid = match read("ppid=") {
        Field::Value(v) => v,
        Field::Missing => return Lookup::Missing,
        Field::Unreadable => return Lookup::Unreadable,
    };
    let Ok(ppid) = ppid.parse() else {
        return Lookup::Unreadable;
    };
    match (read("lstart="), read("comm=")) {
        (Field::Value(start_utc), Field::Value(command)) => Lookup::Found(ProcInfo {
            ppid,
            start_utc,
            command,
        }),
        (Field::Unreadable, _) | (_, Field::Unreadable) => Lookup::Unreadable,
        _ => Lookup::Missing,
    }
}

/// The largest pid this kind of system can number. A pid above it is no process, and `ps` refuses it with a
/// complaint ("process id too large") that must not be read as a table that cannot be read.
#[cfg(target_os = "macos")]
const PID_LIMIT: u32 = 99_999;
#[cfg(not(target_os = "macos"))]
const PID_LIMIT: u32 = 4_194_304;

impl ProcessTable for PsTable {
    fn lookup(&self, pid: u32) -> Lookup {
        if pid > PID_LIMIT {
            return Lookup::Missing;
        }
        lookup_with(&|field| ps(pid, field))
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

    fn reads(ppid: Field, lstart: Field, comm: Field) -> Lookup {
        let mut fields = [ppid, lstart, comm]
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>();
        let by_name = |name: &str| match name {
            "ppid=" => fields[0].take(),
            "lstart=" => fields[1].take(),
            _ => fields[2].take(),
        };
        let cell = std::cell::RefCell::new(by_name);
        lookup_with(&|name| (cell.borrow_mut())(name).expect("each field is read at most once"))
    }

    fn value(v: &str) -> Field {
        Field::Value(v.into())
    }

    #[test]
    fn three_values_are_a_process() {
        let got = reads(
            value("7"),
            value("Fri Nov 6 03:04:05 2026"),
            value("claude"),
        );
        let Lookup::Found(info) = got else {
            panic!("expected a process");
        };
        assert_eq!((info.ppid, info.command.as_str()), (7, "claude"));
    }

    #[test]
    fn a_missing_first_field_is_a_missing_pid_and_an_unreadable_one_is_an_unreadable_table() {
        assert_eq!(
            reads(Field::Missing, value("x"), value("y")),
            Lookup::Missing
        );
        assert_eq!(
            reads(Field::Unreadable, value("x"), value("y")),
            Lookup::Unreadable
        );
    }

    #[test]
    fn a_pid_that_ends_between_the_fields_is_missing_and_a_later_unreadable_field_wins() {
        assert_eq!(
            reads(value("7"), Field::Missing, value("y")),
            Lookup::Missing
        );
        assert_eq!(
            reads(value("7"), value("x"), Field::Missing),
            Lookup::Missing
        );
        assert_eq!(
            reads(value("7"), Field::Missing, Field::Unreadable),
            Lookup::Unreadable
        );
        assert_eq!(
            reads(value("7"), Field::Unreadable, value("y")),
            Lookup::Unreadable
        );
    }

    #[test]
    fn a_parent_that_is_not_a_number_is_unreadable() {
        assert_eq!(
            reads(value("seven"), value("x"), value("y")),
            Lookup::Unreadable
        );
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
