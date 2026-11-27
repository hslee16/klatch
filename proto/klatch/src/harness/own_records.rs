//! Reading a registry directory that klatch owns (the Codex records, #884) without trusting what is in it. Anything
//! that is not a plain file is never read: the file is opened without following a symlink and without blocking (so a
//! FIFO cannot stall the open), and its type, owner and mode are checked on the open descriptor, not on the path, so
//! nothing can be swapped in between the check and the read. At most `MAX_RECORD_BYTES` are read, and at most
//! `MAX_ENTRIES` valid records are read in one pass (and at most `MAX_OPENED` files opened). Only names that look like a
//! record (or a temporary file) are considered, and only a file that parses counts toward the record bound, so junk
//! cannot starve a real file (#1574, #1662); newest first, a future mtime counting as the oldest.

use crate::harness::record::{Record, Registry, is_record_file, parse_record};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// The largest record read. A real one is a few hundred bytes.
pub(crate) const MAX_RECORD_BYTES: u64 = 64 * 1024;

/// The most valid records one pass reads. Records beyond that are left for a later pass (a write looks again).
pub(crate) const MAX_ENTRIES: usize = 256;

/// The most files one pass opens in all, valid or not: records are bounded by `MAX_ENTRIES`, and what is not a record by
/// this (#1662).
pub(crate) const MAX_OPENED: usize = 4096;

/// The most directory entries one pass reads the name of (nothing is opened for them), whatever they are called.
pub(crate) const MAX_SCANNED: usize = 1 << 16;

/// The names in `dir` that `wanted` accepts, at most `scanned` entries looked at. Only the names that are wanted are
/// kept, so an entry of any other name costs a look at its name and nothing more. `None` when `dir` cannot be listed.
fn wanted_names_capped(
    dir: &Path,
    scanned: usize,
    wanted: impl Fn(&str) -> bool,
) -> Option<Vec<String>> {
    let entries = std::fs::read_dir(dir).ok()?;
    Some(
        entries
            .flatten()
            .take(scanned)
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| wanted(n))
            .collect(),
    )
}

fn wanted_names(dir: &Path, wanted: impl Fn(&str) -> bool) -> Option<Vec<String>> {
    wanted_names_capped(dir, MAX_SCANNED, wanted)
}

/// The names of `dir` that look like records (`<pid>.json`), most likely live first: the newest by modification time. A
/// name that cannot be looked at, or whose mtime is in the future (a same-user file can claim any time, #1662), counts
/// as the oldest; the name breaks a tie so the order is stable. `None` when `dir` cannot be listed.
fn ordered_record_names(dir: &Path, scanned: usize, keep_all_below: usize) -> Option<Vec<String>> {
    let mut names = wanted_names_capped(dir, scanned, is_record_file)?;
    if names.len() > keep_all_below {
        let now = std::time::SystemTime::now();
        let modified = |n: &String| {
            std::fs::symlink_metadata(dir.join(n))
                .and_then(|m| m.modified())
                .ok()
                .filter(|t| *t <= now)
        };
        names.sort_by_cached_key(|n| (std::cmp::Reverse(modified(n)), n.clone()));
    }
    Some(names)
}

/// Read the records of `dir` (`<pid>.json`) with `read` giving the text of one name. Candidates are taken newest first
/// (see `ordered_record_names`), but the bound `max_records` counts only records that parse and sit under their own
/// pid, so junk, however fresh, cannot push a valid record out; the junk is bounded separately, at most `max_opened`
/// files opened in one pass. Returns the records ordered by pid and how many opened files were not valid records. `None`
/// when `dir` cannot be listed.
pub(crate) fn scan_records_with(
    dir: &Path,
    scanned: usize,
    max_records: usize,
    max_opened: usize,
    read: impl Fn(&str) -> Option<String>,
) -> Option<(Vec<Record>, usize)> {
    let names = ordered_record_names(dir, scanned, max_records)?;
    let (mut records, mut malformed) = (Vec::new(), 0);
    for name in names.iter().take(max_opened) {
        if records.len() >= max_records {
            break;
        }
        let filed_under: u32 = name.trim_end_matches(".json").parse().unwrap_or(0);
        match read(name).and_then(|t| parse_record(&t)) {
            Some(rec) if rec.pid == filed_under => records.push(rec),
            _ => malformed += 1,
        }
    }
    records.sort_by_key(|r| r.pid);
    Some((records, malformed))
}

/// `scan_records_with` under the module bounds.
pub(crate) fn scan_records(
    dir: &Path,
    read: impl Fn(&str) -> Option<String>,
) -> Option<(Vec<Record>, usize)> {
    scan_records_with(dir, MAX_SCANNED, MAX_ENTRIES, MAX_OPENED, read)
}

/// Whether `meta` is of something this user owns that no one else can write.
fn ours_and_sealed(meta: &std::fs::Metadata, uid: u32) -> bool {
    meta.uid() == uid && meta.permissions().mode() & 0o022 == 0
}

/// Whether `dir` is a real directory (not a symlink) owned by `uid` that group and others cannot write.
pub(crate) fn dir_is_ours(dir: &Path, uid: u32) -> bool {
    std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir() && ours_and_sealed(&m, uid))
}

/// Open `path` for reading without following a symlink and without blocking. `None` when it cannot be opened that way.
pub(crate) fn open_plain(path: &Path) -> Option<std::fs::File> {
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .ok()
}

/// The text of an open file, only when the descriptor is a regular file owned by `uid` that no one else can write and
/// it holds at most `MAX_RECORD_BYTES`. Anything else, an unreadable or non-text file included, is `None`.
pub(crate) fn read_plain(file: std::fs::File, uid: u32) -> Option<String> {
    let meta = file.metadata().ok()?;
    if !ours_and_sealed(&meta, uid) {
        return None;
    }
    read_regular(file)
}

/// The text of an open file, only when the descriptor is a regular file of at most `MAX_RECORD_BYTES`, whoever owns it
/// (the Claude registry is Claude Code's, written under its own umask, so no owner or mode rule is added to it, #1567).
pub(crate) fn read_regular(file: std::fs::File) -> Option<String> {
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > MAX_RECORD_BYTES {
        return None;
    }
    read_capped(file, MAX_RECORD_BYTES)
}

/// The text of `<dir>/<name>` through one open descriptor, under the rules of `read_regular`.
pub(crate) fn read_regular_named(dir: &Path, name: &str) -> Option<String> {
    read_regular(open_plain(&dir.join(name))?)
}

/// At most `cap` bytes of text from `reader`. One byte more is read, so a source that holds more than the file's size
/// said (a file that grew after the check) is still refused; more than `cap`, or not text, is `None`.
pub(crate) fn read_capped(reader: impl Read, cap: u64) -> Option<String> {
    let mut text = String::new();
    reader.take(cap + 1).read_to_string(&mut text).ok()?;
    (text.len() as u64 <= cap).then_some(text)
}

/// The text of `<dir>/<name>` through one open descriptor, under the rules of `read_plain`.
pub(crate) fn read_named(dir: &Path, name: &str, uid: u32) -> Option<String> {
    read_plain(open_plain(&dir.join(name))?, uid)
}

/// The records of `dir`, read only when the directory and the file are ours and sealed (see the module comment).
pub struct OwnRegistry {
    pub dir: PathBuf,
    pub uid: u32,
}

impl Registry for OwnRegistry {
    fn record_for(&self, pid: u32) -> Option<Record> {
        if !dir_is_ours(&self.dir, self.uid) {
            return None;
        }
        let text = read_named(&self.dir, &format!("{pid}.json"), self.uid)?;
        parse_record(&text).filter(|r| r.pid == pid)
    }

    fn records(&self) -> (Vec<Record>, usize) {
        if !dir_is_ours(&self.dir, self.uid) {
            return (Vec::new(), 0);
        }
        scan_records(&self.dir, |name| read_named(&self.dir, name, self.uid))
            .unwrap_or((Vec::new(), 0))
    }
}

/// Whether `name` is a temporary file a writer leaves while it writes (`.<pid>.tmp<n>`).
fn is_temp_name(name: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    name.strip_prefix('.')
        .and_then(|rest| rest.split_once(".tmp"))
        .is_some_and(|(pid, n)| digits(pid) && digits(n))
}

/// Remove temporary files older than `older_than` (by modification time) that a writer killed mid-write left behind. A
/// plain file only (never a symlink, a directory or a FIFO), at most `MAX_ENTRIES` temporary names examined (other names
/// do not count, #1574). A writer holds its
/// temporary file for microseconds, so age stands in for "not open".
pub(crate) fn remove_stale_temps(dir: &Path, older_than: std::time::Duration) {
    let Some(names) = wanted_names(dir, is_temp_name) else {
        return;
    };
    for name in names.iter().take(MAX_ENTRIES) {
        let path = dir.join(name);
        let stale = std::fs::symlink_metadata(&path)
            .ok()
            .filter(|m| m.is_file())
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > older_than);
        if stale {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests;
