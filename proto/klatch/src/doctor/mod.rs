//! `klatch doctor` (#66): a read-only health check. It looks at this binary, the store and its key, the hooks in the
//! Claude Code settings, the session it runs in and the harness registry, and says for each what works, what does not
//! and why, with the fix. It creates nothing (no store, no key, no settings) and never prints a key.

mod hooks;
mod session;
mod store;

use std::io::Write;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Warn,
    Fail,
}

struct Report {
    lines: Vec<(Status, String, Option<String>)>,
}

impl Report {
    fn add(&mut self, status: Status, line: impl Into<String>, fix: Option<&str>) {
        self.lines
            .push((status, line.into(), fix.map(String::from)));
    }

    fn count(&self, status: Status) -> usize {
        self.lines.iter().filter(|l| l.0 == status).count()
    }

    /// Write each line with its tag and, for anything not ok, its fix; then the totals. Returns the failure count.
    fn print(&self, out: &mut dyn Write) -> std::io::Result<usize> {
        for (status, line, fix) in &self.lines {
            let tag = match status {
                Status::Ok => "ok  ",
                Status::Warn => "warn",
                Status::Fail => "FAIL",
            };
            writeln!(out, "{tag}  {line}")?;
            if let Some(fix) = fix.as_ref().filter(|_| *status != Status::Ok) {
                writeln!(out, "      fix: {fix}")?;
            }
        }
        let failures = self.count(Status::Fail);
        writeln!(
            out,
            "\n{} ok, {} warnings, {failures} failures",
            self.count(Status::Ok),
            self.count(Status::Warn)
        )?;
        Ok(failures)
    }
}

/// Run every probe, print the report and return whether nothing failed.
pub fn run(
    project_dir: &Path,
    home: Option<&Path>,
    exe: &Path,
    out: &mut dyn Write,
) -> std::io::Result<bool> {
    let mut r = Report { lines: Vec::new() };
    r.add(
        Status::Ok,
        format!(
            "binary: klatch {} at {}",
            crate::version_text(),
            exe.display()
        ),
        None,
    );
    store::check_store(&mut r, home);
    store::check_archives(&mut r, home);
    store::check_growth(&mut r, home);
    store::check_proof_failures(&mut r, home);
    store::check_recovery_attempts(&mut r, home);
    store::check_modes(&mut r, home);
    store::check_passphrase(&mut r, home);
    session::check_codex(&mut r, home, project_dir);
    session::check_harness(&mut r);
    session::check_process_table(&mut r);
    hooks::check_hooks(&mut r, project_dir, home, exe);
    session::check_session(&mut r);
    session::check_registry(&mut r);
    Ok(r.print(out)? == 0)
}

#[cfg(test)]
mod tests {
    use super::{Report, Status, session, store};
    use klatch::sync::within;
    use std::time::{Duration, Instant};

    #[test]
    fn a_probe_that_answers_in_time_gives_its_answer() {
        assert_eq!(within(Duration::from_secs(5), || 7), Some(7));
    }

    #[test]
    fn a_probe_that_never_answers_is_given_up_on_at_the_limit() {
        let started = Instant::now();
        let got = within(Duration::from_millis(100), || {
            std::thread::sleep(Duration::from_secs(30));
            1
        });
        assert_eq!(got, None);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "waited {:?}",
            started.elapsed()
        );
    }

    fn warnings_for(census: Option<(u64, u64)>) -> Vec<String> {
        let mut r = Report { lines: Vec::new() };
        store::report_proof_failures(&mut r, census);
        r.lines
            .into_iter()
            .inspect(|l| assert!(l.0 == Status::Warn))
            .map(|l| l.1)
            .collect()
    }

    #[test]
    fn the_proof_failure_warning_is_quiet_when_the_census_could_not_be_read_or_is_below_the_level()
    {
        let level = klatch::proof::alarm::CEILING;
        assert!(warnings_for(None).is_empty());
        assert!(warnings_for(Some((level - 1, 9))).is_empty());
        let at_level = warnings_for(Some((level, 9)));
        assert_eq!(at_level.len(), 1);
        assert!(
            at_level[0].contains(&format!("{level} wrong")),
            "{at_level:?}"
        );
        assert!(at_level[0].contains("from 9 terminals"), "{at_level:?}");
    }

    #[test]
    fn unsettled_recovery_attempts_are_a_warning_with_their_count() {
        let lines = |n| {
            let mut r = Report { lines: Vec::new() };
            store::report_recovery_attempts(
                &mut r,
                n,
                std::path::Path::new("/x/rooms.db.recovery-fails"),
            );
            r.lines
        };
        assert!(lines(0).is_empty());
        let two = lines(2);
        assert_eq!(two.len(), 1);
        assert!(two[0].0 == Status::Warn);
        assert!(two[0].1.contains("2 recovery attempts"), "{:?}", two[0].1);
        assert!(two[0].1.contains("rooms.db.recovery-fails"));
    }

    #[test]
    fn the_passphrase_warning_is_only_for_a_readable_store_with_none_set() {
        let lines = |set| {
            let mut r = Report { lines: Vec::new() };
            store::report_passphrase(&mut r, set);
            r.lines
        };
        assert!(lines(Some(true)).is_empty());
        assert!(lines(None).is_empty());
        let unset = lines(Some(false));
        assert_eq!(unset.len(), 1);
        assert!(unset[0].0 == Status::Warn);
        assert!(unset[0].1.contains("no admin passphrase is set"));
        assert!(unset[0].2.as_deref().unwrap().contains("init-passphrase"));
    }

    #[test]
    fn a_missing_ps_is_one_fixed_warning_and_a_present_one_is_quiet() {
        let lines = |notice| {
            let mut r = Report { lines: Vec::new() };
            session::report_process_table(&mut r, notice);
            r.lines
        };
        assert!(lines(None).is_empty());
        let missing = lines(Some(klatch::harness::NO_PROCESS_TABLE));
        assert_eq!(missing.len(), 1);
        assert!(missing[0].0 == Status::Warn);
        assert_eq!(missing[0].1, klatch::harness::NO_PROCESS_TABLE);
    }

    #[test]
    fn doctor_checks_the_process_table_through_the_candidate_list_it_is_given() {
        let lines = |candidates: &[&str]| {
            let mut r = Report { lines: Vec::new() };
            session::check_process_table_in(&mut r, candidates);
            r.lines
        };
        let missing = lines(&["/nonexistent/ps"]);
        assert_eq!(missing.len(), 1);
        assert!(missing[0].0 == Status::Warn);
        assert_eq!(missing[0].1, klatch::harness::NO_PROCESS_TABLE);
        assert!(lines(&klatch::harness::PS_CANDIDATES).is_empty());
    }
}
