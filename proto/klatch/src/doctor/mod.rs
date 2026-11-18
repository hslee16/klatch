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
    store::check_modes(&mut r, home);
    session::check_codex(&mut r, home, project_dir);
    hooks::check_hooks(&mut r, project_dir, home, exe);
    session::check_session(&mut r);
    session::check_registry(&mut r);
    Ok(r.print(out)? == 0)
}

#[cfg(test)]
mod tests {
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
}
