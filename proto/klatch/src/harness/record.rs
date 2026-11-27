//! The harness registry: `<pid>.json` files, one per live session. Only the `.json` files are read; the
//! `.key` files beside them hold secrets and are never opened.

use crate::harness::own_records::{Scan, read_regular_named, scan_records};
use crate::harness::process::squash;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub pid: u32,
    pub session_id: String,
    pub cwd: String,
    pub name: Option<String>,
    /// UTC start time of the harness process, whitespace collapsed.
    pub proc_start: String,
    pub pid_domain: String,
}

/// `None` when the text is not a record with a pid, session id, start time and pid domain. Unknown fields are
/// ignored so a newer harness version still parses.
pub fn parse_record(json: &str) -> Option<Record> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let text = |k: &str| v.get(k)?.as_str().map(squash).filter(|s| !s.is_empty());
    Some(Record {
        pid: u32::try_from(v.get("pid")?.as_u64()?).ok()?,
        session_id: text("sessionId")?,
        cwd: v
            .get("cwd")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string(),
        name: text("name"),
        proc_start: text("procStart")?,
        pid_domain: text("pidDomain")?,
    })
}

pub trait Registry {
    /// The record filed under this pid, if there is a well formed one.
    fn record_for(&self, pid: u32) -> Option<Record>;

    /// Every well formed record, ordered by pid, and how many files looked like records but did not parse. A registry
    /// that cannot be listed (a fake, a missing directory) has none.
    fn records(&self) -> (Vec<Record>, usize) {
        (Vec::new(), 0)
    }
}

/// Whether a file name is exactly `<pid>.json` with the pid written canonically (no leading zero, so `05000.json` is
/// never filed under pid 5000 beside `5000.json`, #1692). Nothing else in the registry directory is ever opened: the
/// `<pid>.<hash>.key` files hold secrets.
pub fn is_record_file(name: &str) -> bool {
    name.strip_suffix(".json").is_some_and(|pid| {
        pid.bytes().all(|b| b.is_ascii_digit())
            && pid.as_bytes().first().is_some_and(|b| *b != b'0')
    })
}

/// The registry directory on disk (Claude Code: `~/.claude/sessions`).
pub struct FsRegistry {
    pub dir: PathBuf,
}

impl FsRegistry {
    /// `records`, with the records of the pids in `alive` (the pids of a readable process table, `None` when there is
    /// none) read before the others, whatever their age: a forged record for a pid that does not run cannot push a
    /// record of a running process out of the bound (#1692).
    ///
    /// The third value is whether a bound cut the listing (#1706): the records are then a part of the directory, and a
    /// caller that acts on what is absent from them, or on one record among several, acts on an incomplete view.
    pub fn records_alive(&self, alive: Option<&HashSet<u32>>) -> (Vec<Record>, usize, bool) {
        self.scan_alive(alive).map_or((Vec::new(), 0, false), |s| {
            (s.records, s.malformed, s.truncated)
        })
    }

    /// The whole of one pass (see `records_alive`), the claims of dead pids read past the record bound included
    /// (#1712). `None` when the directory cannot be listed.
    pub(crate) fn scan_alive(&self, alive: Option<&HashSet<u32>>) -> Option<Scan> {
        // through one descriptor opened without following a symlink or blocking on a FIFO, size capped (#1567)
        scan_records(&self.dir, alive, |name| read_regular_named(&self.dir, name))
    }
}

impl Registry for FsRegistry {
    fn records(&self) -> (Vec<Record>, usize) {
        let (records, malformed, _) = self.records_alive(None);
        (records, malformed)
    }

    fn record_for(&self, pid: u32) -> Option<Record> {
        // only `<pid>.json`; the `<pid>.<hash>.key` files are never opened
        let text = read_regular_named(&self.dir, &format!("{pid}.json"))?;
        parse_record(&text).filter(|r| r.pid == pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const GOOD: &str = r#"{"pid":19479,"sessionId":"abc-123","cwd":"/work/x","procStart":"Fri Nov  6 03:04:05 2026","pidDomain":"darwin","name":"klatch-1b","version":"9.9.9","somethingNew":{"a":1}}"#;

    #[test]
    fn parses_a_record_and_ignores_unknown_fields() {
        let r = parse_record(GOOD).unwrap();
        assert_eq!(r.pid, 19479);
        assert_eq!(r.session_id, "abc-123");
        assert_eq!(r.cwd, "/work/x");
        assert_eq!(r.name.as_deref(), Some("klatch-1b"));
        assert_eq!(
            r.proc_start, "Fri Nov 6 03:04:05 2026",
            "whitespace collapsed"
        );
        assert_eq!(r.pid_domain, "darwin");
    }

    #[test]
    fn a_missing_name_is_fine() {
        let j = GOOD.replace(r#""name":"klatch-1b","#, "");
        assert_eq!(parse_record(&j).unwrap().name, None);
    }

    #[test]
    fn malformed_records_are_none() {
        for bad in [
            "",
            "not json",
            "[]",
            r#"{"pid":1}"#,
            &GOOD.replace("\"pid\":19479", "\"pid\":\"x\""),
            &GOOD.replace("\"sessionId\":\"abc-123\"", "\"sessionId\":\"\""),
            &GOOD.replace("\"procStart\"", "\"procStartX\""),
            &GOOD.replace("\"pidDomain\"", "\"pidDomainX\""),
        ] {
            assert_eq!(parse_record(bad), None, "{bad}");
        }
    }

    fn tmp(tag: &str) -> crate::store::test_dir::TestDir {
        crate::store::test_dir::TestDir::new(&format!("reg-{tag}"))
    }

    #[test]
    fn reads_the_record_for_a_pid() {
        let d = tmp("read");
        std::fs::write(d.join("19479.json"), GOOD).unwrap();
        let reg = FsRegistry {
            dir: d.to_path_buf(),
        };
        assert_eq!(reg.record_for(19479).unwrap().session_id, "abc-123");
        assert_eq!(reg.record_for(1), None);
    }

    #[test]
    fn a_record_filed_under_another_pid_is_not_believed() {
        let d = tmp("mismatch");
        std::fs::write(d.join("555.json"), GOOD).unwrap();
        assert_eq!(
            FsRegistry {
                dir: d.to_path_buf()
            }
            .record_for(555),
            None
        );
    }

    #[test]
    fn never_opens_a_key_file() {
        let d = tmp("keys");
        std::fs::write(d.join("19479.json"), GOOD).unwrap();
        let key = d.join("19479.deadbeef.key");
        std::fs::write(&key, "secret").unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o000)).unwrap();
        // an unreadable key file proves it is not opened; the lookup still works
        assert!(
            FsRegistry {
                dir: d.to_path_buf()
            }
            .record_for(19479)
            .is_some()
        );
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    /// Run `f` on a thread and give up after a few seconds, so a reader that blocks fails the test instead of hanging
    /// the run (#1616).
    fn within_seconds<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .expect("the registry read blocked")
    }

    fn reg_at(d: &std::path::Path) -> FsRegistry {
        FsRegistry {
            dir: d.to_path_buf(),
        }
    }

    #[test]
    fn a_symlinked_record_is_refused() {
        let d = tmp("symlink");
        let real = d.join("real.txt");
        std::fs::write(&real, GOOD).unwrap();
        std::os::unix::fs::symlink(&real, d.join("19479.json")).unwrap();
        let dir = d.to_path_buf();
        assert_eq!(reg_at(&dir).record_for(19479), None);
        let (records, malformed) = reg_at(&dir).records();
        assert!(records.is_empty(), "a symlink is not evidence");
        assert_eq!(malformed, 1, "it is counted as a record that did not read");
    }

    #[test]
    fn a_fifo_named_like_a_record_does_not_hang_the_reader() {
        let d = tmp("fifo");
        let fifo = std::ffi::CString::new(d.join("19479.json").to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        std::fs::write(d.join("20000.json"), GOOD.replace("19479", "20000")).unwrap();
        let dir = d.to_path_buf();
        let (one, (records, malformed)) =
            within_seconds(move || (reg_at(&dir).record_for(19479), reg_at(&dir).records()));
        assert_eq!(one, None);
        assert_eq!(malformed, 1);
        assert_eq!(records.len(), 1, "the good record beside it is still read");
    }

    #[test]
    fn an_oversized_record_is_refused() {
        let d = tmp("big");
        let padding = " ".repeat(crate::harness::own_records::MAX_RECORD_BYTES as usize);
        std::fs::write(d.join("19479.json"), format!("{GOOD}{padding}")).unwrap();
        let reg = reg_at(&d);
        assert_eq!(reg.record_for(19479), None);
        assert_eq!(reg.records(), (Vec::new(), 1));
    }

    #[test]
    fn a_record_under_the_cap_is_still_read_whatever_its_mode() {
        let d = tmp("mode");
        std::fs::write(d.join("19479.json"), GOOD).unwrap();
        std::fs::set_permissions(d.join("19479.json"), std::fs::Permissions::from_mode(0o664))
            .unwrap();
        assert!(reg_at(&d).record_for(19479).is_some());
        assert_eq!(reg_at(&d).records().0.len(), 1);
    }

    #[test]
    fn at_most_the_entry_bound_of_records_are_read_in_one_pass() {
        let d = tmp("bound");
        let n = crate::harness::own_records::MAX_ENTRIES + 20;
        for pid in 1000..1000 + n {
            let json = GOOD.replace("19479", &pid.to_string());
            std::fs::write(d.join(format!("{pid}.json")), json).unwrap();
        }
        let (records, _) = reg_at(&d).records();
        assert_eq!(records.len(), crate::harness::own_records::MAX_ENTRIES);
    }

    #[test]
    fn files_that_are_not_records_do_not_use_up_the_entry_bound() {
        let d = tmp("noise");
        for i in 0..crate::harness::own_records::MAX_ENTRIES + 10 {
            std::fs::write(d.join(format!("noise-{i}.txt")), "x").unwrap();
        }
        std::fs::write(d.join("19479.json"), GOOD).unwrap();
        assert_eq!(
            reg_at(&d).records(),
            (vec![reg_at(&d).record_for(19479).unwrap()], 0)
        );
    }

    #[test]
    fn with_more_records_than_the_bound_a_live_session_is_not_hidden_by_old_ones() {
        let d = tmp("newest");
        let max = crate::harness::own_records::MAX_ENTRIES;
        let n = max + 44;
        let now = std::time::SystemTime::now();
        for i in 0..n {
            let pid = 1000 + i;
            let file = d.join(format!("{pid}.json"));
            std::fs::write(&file, GOOD.replace("19479", &pid.to_string())).unwrap();
            // a shuffled age, whole minutes apart: directory order says nothing about it
            let rank = (i * 7919) % n;
            let when = now - std::time::Duration::from_secs(100_000 - 60 * rank as u64);
            std::fs::File::options()
                .write(true)
                .open(&file)
                .unwrap()
                .set_modified(when)
                .unwrap();
        }
        let (records, _) = reg_at(&d).records();
        assert_eq!(records.len(), max);
        let newest: Vec<u32> = (0..n)
            .filter(|i| (i * 7919) % n >= n - max)
            .map(|i| 1000 + i as u32)
            .collect();
        let got: Vec<u32> = records.iter().map(|r| r.pid).collect();
        assert_eq!(got, newest, "a live (newest) record was not read");
    }

    #[test]
    fn junk_entries_do_not_hide_a_record() {
        let d = tmp("junk");
        for i in 0..3000 {
            std::fs::write(d.join(format!("junk-{i}.txt")), "x").unwrap();
        }
        std::fs::write(d.join("19479.json"), GOOD).unwrap();
        assert_eq!(reg_at(&d).records().0.len(), 1);
    }
}
