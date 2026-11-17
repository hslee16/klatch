//! What klatch can see of Codex without logging in, calling a model or changing anything (#544, #545; RESEARCH-NOTES
//! 49.3). Codex skips a hook that was never trusted and, in `codex exec`, says nothing, so a klatch hook there would
//! look healthy and deliver nothing. Doctor and init read the Codex files and say so. Klatch never passes
//! `--dangerously-bypass-hook-trust` and never writes a trust record (the hash input is not known).

use serde_json::Value;
use std::collections::HashSet;
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

/// A hook event as Codex spells it in a trust record: `PreToolUse` is `pre_tool_use`.
fn snake(event: &str) -> String {
    let mut out = String::new();
    for (i, c) in event.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Where each klatch hook sits in a Codex hooks file: its event, the index of its matcher group and its index in
/// that group, which is how Codex names it in a trust record.
fn klatch_slots(hooks: &Value) -> Vec<(String, usize, usize)> {
    let Some(events) = hooks.get("hooks").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut slots = Vec::new();
    for (event, groups) in events {
        for (n, group) in groups.as_array().into_iter().flatten().enumerate() {
            slots.extend(klatch_in_group(group).map(|m| (event.clone(), n, m)));
        }
    }
    slots
}

/// The indexes, within one matcher group, of the hooks that run a klatch command.
fn klatch_in_group(group: &Value) -> impl Iterator<Item = usize> + '_ {
    let list = group.get("hooks").and_then(Value::as_array);
    list.into_iter()
        .flatten()
        .enumerate()
        .filter(|(_, hook)| {
            let cmd = hook.get("command").and_then(Value::as_str);
            cmd.is_some_and(crate::init::is_klatch_command)
        })
        .map(|(m, _)| m)
}

/// Whether a config line sets a real trust hash: `trusted_hash` with a `sha256:` value.
fn is_trust_hash(line: &str) -> bool {
    line.starts_with("trusted_hash") && line.contains("sha256:")
}

/// The `[hooks.state."<key>"]` sections of a Codex config that carry a `trusted_hash`. A section with no hash is not
/// trust. Plain line scanning, because the form is fixed and klatch carries no TOML parser for one table.
fn trusted_keys(config: &str) -> HashSet<String> {
    let mut trusted = HashSet::new();
    let mut section: Option<String> = None;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line
                .strip_prefix("[hooks.state.\"")
                .and_then(|r| r.strip_suffix("\"]"))
                .map(String::from);
        } else if let Some(key) = section.as_ref().filter(|_| is_trust_hash(line)) {
            trusted.insert(key.clone());
        }
    }
    trusted
}

/// The finding about klatch's hooks in Codex's user hooks file, or none when there are no klatch hooks there.
pub fn hook_findings(codex_home: &Path) -> Option<Finding> {
    let file = codex_home.join("hooks.json");
    let text = std::fs::read_to_string(&file).ok()?;
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return Some(finding(
            false,
            format!(
                "codex hooks: {} is not valid JSON, so klatch cannot tell whether its hooks are trusted",
                file.display()
            ),
            None,
        ));
    };
    let slots = klatch_slots(&value);
    if slots.is_empty() {
        return None;
    }
    let config = codex_home.join("config.toml");
    let trusted = std::fs::read_to_string(&config)
        .map(|t| trusted_keys(&t))
        .unwrap_or_default();
    let untrusted: Vec<String> = slots
        .iter()
        .filter(|(event, n, m)| {
            let key = format!("{}:{}:{n}:{m}", file.display(), snake(event));
            !trusted.contains(&key)
        })
        .map(|(event, ..)| event.clone())
        .collect();
    if untrusted.is_empty() {
        return Some(finding(
            true,
            format!(
                "codex hooks: klatch's hooks in {} have a trust record in {} (whether it matches the current definition cannot be checked; Codex skips a changed hook until it is trusted again)",
                file.display(),
                config.display()
            ),
            None,
        ));
    }
    Some(finding(
        false,
        format!(
            "codex hooks: {} klatch hook(s) in {} are not trusted by Codex ({}) and will be skipped silently in `codex exec`",
            untrusted.len(),
            file.display(),
            untrusted.join(", ")
        ),
        Some(
            "open Codex and review the hooks in /hooks to trust them; klatch never passes --dangerously-bypass-hook-trust and never writes the trust record itself",
        ),
    ))
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
pub fn init_notice(codex_home: &Path, out: &mut dyn std::io::Write) -> std::io::Result<()> {
    if let Some(f) = hook_findings(codex_home).filter(|f| !f.ok) {
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
    fn events_are_spelled_the_way_codex_keys_a_trust_record() {
        assert_eq!(snake("PreToolUse"), "pre_tool_use");
        assert_eq!(snake("SessionStart"), "session_start");
        assert_eq!(snake("PostToolUseFailure"), "post_tool_use_failure");
    }

    #[test]
    fn only_a_section_with_a_sha256_hash_is_trust() {
        let cfg = "[hooks.state.\"a\"]\ntrusted_hash = \"sha256:1\"\n\n[hooks.state.\"b\"]\n\n[other]\ntrusted_hash = \"sha256:2\"\n[hooks.state.\"c\"]\ntrusted_hash = \"\"\n";
        let t = trusted_keys(cfg);
        assert!(t.contains("a"));
        assert!(!t.contains("b") && !t.contains("c") && !t.contains("other"));
    }

    #[test]
    fn a_usage_line_for_the_top_level_is_not_a_queue_command() {
        assert!(!has_queue("Usage: codex [OPTIONS] [PROMPT]\n"));
        assert!(has_queue(
            "Queue a message\n\nUsage: codex queue [OPTIONS]\n"
        ));
    }
}
