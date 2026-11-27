//! Reading settings files for klatch hooks: bounded, read only, never following a link out of the project.

use serde_json::Value;
use std::path::{Path, PathBuf};

/// Most directories walked upward and most bytes read from one settings file.
const MAX_LEVELS: usize = 64;
const MAX_BYTES: u64 = 1024 * 1024;

/// What one settings file held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Seen {
    Absent,
    NoHooks,
    Hooks,
}

/// A hook command klatch wrote (or a hand-written one of the same form); `klatch init` recognises its own the same way.
pub fn command_is_klatch(cmd: &str) -> bool {
    cmd.contains(" hook --event ") && cmd.contains("klatch")
}

/// The repository root above `cwd` (the nearest directory with a `.git`), else `cwd` itself.
pub(super) fn project_root(cwd: &Path) -> PathBuf {
    cwd.ancestors()
        .take(MAX_LEVELS)
        .find(|d| d.join(".git").exists())
        .unwrap_or(cwd)
        .to_path_buf()
}

/// `cwd` and each parent up to and including `root`.
pub(super) fn dirs_up_to(cwd: &Path, root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for d in cwd.ancestors().take(MAX_LEVELS) {
        dirs.push(d.to_path_buf());
        if d == root {
            break;
        }
    }
    dirs
}

/// What `file` holds, or None when that cannot be told with certainty: unreadable, too big, not a file, malformed, or a
/// link that leaves `within`.
pub(super) fn look(file: &Path, within: &Path) -> Option<Seen> {
    let meta = match std::fs::symlink_metadata(file) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(Seen::Absent),
        Err(_) => return None,
    };
    if meta.file_type().is_symlink() {
        let target = std::fs::canonicalize(file).ok()?;
        if !target.starts_with(std::fs::canonicalize(within).ok()?) {
            return None;
        }
    }
    let meta = std::fs::metadata(file).ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return None;
    }
    let value: Value = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
    Some(if has_klatch_hook(&value) {
        Seen::Hooks
    } else {
        Seen::NoHooks
    })
}

fn has_klatch_hook(settings: &Value) -> bool {
    let Some(events) = settings.get("hooks").and_then(Value::as_object) else {
        return false;
    };
    events
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|m| m.get("hooks").and_then(Value::as_array))
        .flatten()
        .filter_map(|h| h.get("command").and_then(Value::as_str))
        .any(command_is_klatch)
}
