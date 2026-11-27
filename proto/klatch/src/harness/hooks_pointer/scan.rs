//! Reading settings files for klatch hooks: bounded, read only, never following a link out of the project.

use serde_json::Value;
use std::path::{Path, PathBuf};

/// Most directories walked upward and most bytes read from one settings file.
const MAX_LEVELS: usize = 64;
pub(super) const MAX_BYTES: u64 = 1024 * 1024;

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

/// What `file` holds, or None when that cannot be told with certainty: unreadable, too big, not a regular file,
/// malformed, a link that leaves `within`, or (when `follow_dir` is false) a `.claude` directory that is itself a link.
/// The file is opened once, without blocking, and every check after that is on the open handle, so a swap between a
/// check and the read (a FIFO, a huge file) cannot hang or outgrow the cap.
pub(super) fn look(file: &Path, within: &Path, follow_dir: bool) -> Option<Seen> {
    if !follow_dir {
        if let Some(dir) = file.parent() {
            match std::fs::symlink_metadata(dir) {
                Ok(m) if m.file_type().is_symlink() => return None,
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(Seen::Absent),
                Err(_) => return None,
            }
        }
    }
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
    let handle = open_nonblocking(file).ok()?;
    let meta = handle.metadata().ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return None;
    }
    let value: Value = serde_json::from_str(&read_capped(handle)?).ok()?;
    Some(if has_klatch_hook(&value) {
        Seen::Hooks
    } else {
        Seen::NoHooks
    })
}

#[cfg(unix)]
fn open_nonblocking(file: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    // O_NONBLOCK so opening a FIFO with no writer returns at once instead of waiting for one
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(file)
}

#[cfg(not(unix))]
fn open_nonblocking(file: &Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(file)
}

/// All of `r` as text, or None when it is not valid UTF-8, cannot be read, or holds more than `MAX_BYTES`.
pub(super) fn read_capped(r: impl std::io::Read) -> Option<String> {
    use std::io::Read;
    let mut text = String::new();
    r.take(MAX_BYTES + 1).read_to_string(&mut text).ok()?;
    (text.len() as u64 <= MAX_BYTES).then_some(text)
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
