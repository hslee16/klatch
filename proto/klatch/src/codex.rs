//! What klatch can see of Codex without logging in, calling a model or changing anything (#544, #545; RESEARCH-NOTES
//! 49.3). Codex skips a hook that was never trusted and, in `codex exec`, says nothing, so a klatch hook there would
//! look healthy and deliver nothing. Doctor and init read the Codex files and say so (`hooks`). Klatch never passes
//! `--dangerously-bypass-hook-trust` and never writes a trust record.

mod hooks;

pub use hooks::hook_findings;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// One thing found: whether it is fine, what it says and what to do about it.
pub struct Finding {
    pub ok: bool,
    pub line: String,
    pub fix: Option<String>,
}

fn finding(ok: bool, line: String, fix: Option<&str>) -> Finding {
    Finding {
        ok,
        line,
        fix: fix.map(String::from),
    }
}

/// Where Codex keeps its user configuration: `CODEX_HOME`, else `.codex` under the home directory.
pub fn codex_home(env: Option<std::ffi::OsString>, home: Option<&Path>) -> Option<PathBuf> {
    match env.filter(|e| !e.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => home.map(|h| h.join(".codex")),
    }
}

/// How long `codex` may take to answer a help request.
const CODEX_PATIENCE: Duration = Duration::from_secs(10);

/// Run `codex` with `args` on a thread with a time limit: `None` when it did not answer in time, `Err` when it could
/// not be started. Only help and version requests are made, which need no login and write nothing.
fn run_codex(args: &'static [&'static str]) -> Option<std::io::Result<String>> {
    klatch::sync::within(CODEX_PATIENCE, move || {
        Command::new("codex")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout).into_owned()
                    + &String::from_utf8_lossy(&o.stderr)
            })
    })
}

/// Whether `codex queue --help` is the usage of a real subcommand. When there is none, Codex prints its top level
/// usage (`Usage: codex [OPTIONS] [PROMPT]`), so only a usage line naming `codex queue` counts.
fn has_queue(help: &str) -> bool {
    help.lines()
        .map(str::trim)
        .any(|l| l.starts_with("Usage:") && l.contains("codex queue"))
}

/// The first line of `codex --version`, safe to print.
fn version_of(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    line.chars().filter(|c| !c.is_control()).take(60).collect()
}

/// Whether a Codex wake path through `codex queue` exists here, found by running `codex queue --help` (#545). A
/// version number says nothing: the command is in newer binaries and not in 0.147.0.
pub fn queue_finding() -> Finding {
    let Some(Ok(version)) = run_codex(&["--version"]) else {
        return finding(
            true,
            "codex: not installed on the PATH (nothing to check)".into(),
            None,
        );
    };
    let version = version_of(&version);
    match run_codex(&["queue", "--help"]) {
        Some(Ok(help)) if has_queue(&help) => finding(
            true,
            format!(
                "codex queue: this codex has a queue subcommand (found by running `codex queue --help`, not by version; {version}); it takes a thread id and text with no sender and no acknowledgement, so exit 0 is not proof of delivery"
            ),
            None,
        ),
        Some(Ok(_)) => finding(
            true,
            format!(
                "codex queue: this codex has no queue subcommand (`codex queue --help` shows no usage for it; {version}), so no Codex wake path exists here; hooks are the only delivery"
            ),
            None,
        ),
        _ => finding(
            false,
            format!(
                "codex queue: `codex queue --help` did not answer within {} seconds ({version})",
                CODEX_PATIENCE.as_secs()
            ),
            None,
        ),
    }
}

/// What `init` says about Codex after it has done its own work: only an untrusted klatch hook, as a note. Reads
/// the Codex files and writes nothing.
pub fn init_notice(
    codex_home: &Path,
    project_dir: &Path,
    out: &mut dyn std::io::Write,
) -> std::io::Result<()> {
    for f in hook_findings(codex_home, Some(project_dir))
        .into_iter()
        .filter(|f| !f.ok)
    {
        writeln!(out, "note: {}", f.line)?;
        if let Some(fix) = f.fix {
            writeln!(out, "      fix: {fix}")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_usage_line_for_the_top_level_is_not_a_queue_command() {
        assert!(!has_queue("Usage: codex [OPTIONS] [PROMPT]\n"));
        assert!(has_queue(
            "Queue a message\n\nUsage: codex queue [OPTIONS]\n"
        ));
    }

    #[test]
    fn the_usage_line_a_real_codex_prints_is_recognised() {
        // `codex queue --help` of codex-cli 0.169.0, run 2026-11-17 (RESEARCH-NOTES 49.7)
        let real = "Queue a message for an existing session\n\nUsage: codex queue [OPTIONS] --thread <THREAD> --message <TEXT>\n\nOptions:\n      --thread <THREAD>\n          Session UUID or exact session name\n";
        assert!(has_queue(real));
    }
}
