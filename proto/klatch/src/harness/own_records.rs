//! Reading a registry directory that klatch owns (the Codex records, #884) without trusting what is in it. Anything
//! that is not a plain file is never read: the file is opened without following a symlink and without blocking (so a
//! FIFO cannot stall the open), and its type, owner and mode are checked on the open descriptor, not on the path, so
//! nothing can be swapped in between the check and the read. At most `MAX_RECORD_BYTES` are read, and at most
//! `MAX_ENTRIES` valid records are read in one pass (and at most `MAX_OPENED` files opened). Only names that look like a
//! record (or a temporary file) are considered, and only a file that parses counts toward the record bound, so junk
//! cannot starve a real file (#1574, #1662); the pids of running processes first when a process table was read, then
//! newest first, an mtime far in the future counting as the oldest (#1692).

use crate::harness::record::{Record, Registry, is_record_file, parse_record};
use std::collections::HashSet;
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

/// The names in `dir` that `wanted` accepts, at most `scanned` entries looked at, and whether more entries were left
/// unlooked at. Only the names that are wanted are kept, so an entry of any other name costs a look at its name and
/// nothing more. `None` when `dir` cannot be listed.
fn wanted_names_capped(
    dir: &Path,
    scanned: usize,
    wanted: impl Fn(&str) -> bool,
) -> Option<(Vec<String>, bool)> {
    let mut entries = std::fs::read_dir(dir).ok()?.flatten();
    let names = entries
        .by_ref()
        .take(scanned)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| wanted(n))
        .collect();
    Some((names, entries.next().is_some()))
}

fn wanted_names(dir: &Path, wanted: impl Fn(&str) -> bool) -> Option<Vec<String>> {
    wanted_names_capped(dir, MAX_SCANNED, wanted).map(|(names, _)| names)
}

/// How far ahead of now a modification time may be and still be believed: a clock stepped back (NTP) leaves a live
/// record ahead of the clock by the size of the step. Further ahead than this, a same-user file is claiming any time it
/// likes (#1662) and counts as the oldest.
const CLOCK_SKEW: std::time::Duration = std::time::Duration::from_secs(86_400);

/// The names of `dir` that look like records (`<pid>.json`), most likely live first: those whose pid is in `alive` (the
/// pids of a readable process table; `None` when there is none, which changes nothing), then the newest by modification
/// time. A name that cannot be looked at, or whose mtime is further in the future than `CLOCK_SKEW`, counts as the
/// oldest; one a little ahead counts as now. The name breaks a tie so the order is stable. `None` when `dir` cannot be
/// listed. Also whether the directory held more entries than `scanned`.
fn ordered_record_names(
    dir: &Path,
    scanned: usize,
    keep_all_below: usize,
    alive: Option<&HashSet<u32>>,
) -> Option<(Vec<String>, bool)> {
    let (mut names, cut) = wanted_names_capped(dir, scanned, is_record_file)?;
    if names.len() > keep_all_below {
        let now = std::time::SystemTime::now();
        let modified = |n: &String| {
            std::fs::symlink_metadata(dir.join(n))
                .and_then(|m| m.modified())
                .ok()
                .filter(|t| *t <= now + CLOCK_SKEW)
                .map(|t| t.min(now))
        };
        let running = |n: &String| {
            let pid = n.trim_end_matches(".json").parse::<u32>().ok();
            alive.is_some_and(|set| pid.is_some_and(|p| set.contains(&p)))
        };
        names.sort_by_cached_key(|n| {
            (
                std::cmp::Reverse(running(n)),
                std::cmp::Reverse(modified(n)),
                n.clone(),
            )
        });
    }
    Some((names, cut))
}

/// Read the records of `dir` (`<pid>.json`) with `read` giving the text of one name. Candidates are taken running pids
/// first, then newest (see `ordered_record_names`), but the bound `max_records` counts only records that parse and sit
/// under their own pid, so junk, however fresh, cannot push a valid record out; the junk is bounded separately, at most `max_opened`
/// files opened in one pass. Returns the records ordered by pid and how many opened files were not valid records. `None`
/// when `dir` cannot be listed.
#[cfg(test)]
pub(crate) fn scan_records_with(
    dir: &Path,
    scanned: usize,
    max_records: usize,
    max_opened: usize,
    alive: Option<&HashSet<u32>>,
    read: impl Fn(&str) -> Option<String>,
) -> Option<(Vec<Record>, usize)> {
    let s = scan_records_bounded(dir, scanned, max_records, max_opened, alive, read)?;
    Some((s.records, s.malformed))
}

/// Whether `unread` (names left unread by a bound) may hold a record that matters: any of them when there is no process
/// table, else one whose pid runs (the order puts those first, so what is left is of pids that do not run).
///
/// An unread dead-pid record is not nothing (#1712): a dead member's real record is the second (Gone) claim that turns a
/// forged alive record of the same session id from an extension into `None`. The scan therefore keeps opening
/// dead-pid names past the record bound for their session ids (`Scan::dead_sessions`), and says when it could not
/// (`Scan::dead_unread`). A cut is reported here only as far as running pids go.
fn cut_running(unread: &[String], alive: Option<&HashSet<u32>>) -> bool {
    match alive {
        None => !unread.is_empty(),
        Some(set) => unread.iter().any(|n| {
            n.trim_end_matches(".json")
                .parse::<u32>()
                .is_ok_and(|p| set.contains(&p))
        }),
    }
}

/// What one pass over a registry directory found.
pub(crate) struct Scan {
    /// The valid records, ordered by pid.
    pub(crate) records: Vec<Record>,
    /// How many opened files were not valid records.
    pub(crate) malformed: usize,
    /// A bound stopped the pass with names left unread that may matter (see `cut_running`): the records are a part of
    /// the directory, not all of it. Names of pids that do not run, left unread, do not count.
    pub(crate) truncated: bool,
    /// The session ids of valid records of pids that do not run, read past the record bound (#1712): only the claim
    /// is kept. A dead member's real record is a second (Gone) claim whatever the flood of running pids ahead of it.
    pub(crate) dead_sessions: Vec<String>,
    /// Names of pids that do not run were still unread when the open bound was reached: one of them may hold such a
    /// claim, so a caller must not act on an alive answer (#1712).
    pub(crate) dead_unread: bool,
}

/// `scan_records_with` that also says whether a bound cut the pass.
pub(crate) fn scan_records_bounded(
    dir: &Path,
    scanned: usize,
    max_records: usize,
    max_opened: usize,
    alive: Option<&HashSet<u32>>,
    read: impl Fn(&str) -> Option<String>,
) -> Option<Scan> {
    let (names, truncated) = ordered_record_names(dir, scanned, max_records, alive)?;
    let (mut records, mut malformed, mut visited) = (Vec::new(), 0, 0);
    for name in names.iter().take(max_opened) {
        if records.len() >= max_records {
            break;
        }
        visited += 1;
        let filed_under: u32 = name.trim_end_matches(".json").parse().unwrap_or(0);
        match read(name).and_then(|t| parse_record(&t)) {
            Some(rec) if rec.pid == filed_under => records.push(rec),
            _ => malformed += 1,
        }
    }
    records.sort_by_key(|r| r.pid);
    let truncated = truncated || cut_running(&names[visited..], alive);
    // what is left is of pids that do not run (else `truncated` is set): their claims still count, so keep opening
    // them for the session id alone, up to the open bound (#1712)
    let mut dead_sessions = Vec::new();
    if !truncated && alive.is_some() {
        for name in names[visited..]
            .iter()
            .take(max_opened.saturating_sub(visited))
        {
            let filed_under: u32 = name.trim_end_matches(".json").parse().unwrap_or(0);
            if let Some(rec) = read(name)
                .and_then(|t| parse_record(&t))
                .filter(|r| r.pid == filed_under)
            {
                dead_sessions.push(rec.session_id);
            }
            visited += 1;
        }
    }
    Some(Scan {
        records,
        malformed,
        truncated,
        dead_sessions,
        dead_unread: !truncated && alive.is_some() && visited < names.len(),
    })
}

/// `scan_records_with` under the module bounds.
pub(crate) fn scan_records(
    dir: &Path,
    alive: Option<&HashSet<u32>>,
    read: impl Fn(&str) -> Option<String>,
) -> Option<Scan> {
    scan_records_bounded(dir, MAX_SCANNED, MAX_ENTRIES, MAX_OPENED, alive, read)
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
        scan_records(&self.dir, None, |name| {
            read_named(&self.dir, name, self.uid)
        })
        .map_or((Vec::new(), 0), |s| (s.records, s.malformed))
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
mod rank_tests;

#[cfg(test)]
mod tests;
