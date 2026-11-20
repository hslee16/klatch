//! The real `Host` for the liveness probe: `ps` for the table, the registry file for the turn state, and file
//! metadata (never content) for the transcript and the background-task output.

use crate::harness::probe::{FileMeta, Host, ProcRow, Turn, parse_turn};
use crate::harness::pstime::{parse_cpu_time, parse_elapsed};
use crate::harness::record::Record;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct SysHost {
    /// `~/.claude/sessions`
    pub registry_dir: PathBuf,
    /// `~/.claude/projects`: one directory per working directory, one `<session id>.jsonl` per session.
    pub projects_dir: PathBuf,
    /// The harness's scratch root (`/tmp/claude-<uid>`): `<encoded cwd>/<session id>/tasks/` holds task output.
    pub task_root: PathBuf,
}

impl SysHost {
    /// The usual places under a home directory. The task root is the first of `/private/tmp/claude-<uid>` and
    /// `/tmp/claude-<uid>` that exists, with the uid taken from the home directory's owner.
    pub fn standard(home: &Path) -> SysHost {
        use std::os::unix::fs::MetadataExt;
        let uid = std::fs::metadata(home).map(|m| m.uid()).unwrap_or(0);
        let roots = [
            format!("/private/tmp/claude-{uid}"),
            format!("/tmp/claude-{uid}"),
        ];
        let task_root = roots
            .iter()
            .map(PathBuf::from)
            .find(|p| p.is_dir())
            .unwrap_or_else(|| PathBuf::from(&roots[1]));
        SysHost {
            registry_dir: home.join(".claude/sessions"),
            projects_dir: home.join(".claude/projects"),
            task_root,
        }
    }
}

/// How the harness names a directory after a working directory: every character that is not a letter or digit
/// becomes `-`.
pub(crate) fn encode_cwd(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// A session id is used as a path component, so anything but letters, digits, `-` and `_` is refused.
fn safe_component(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn meta_of(path: &Path) -> Option<FileMeta> {
    let m = std::fs::metadata(path).ok()?;
    let modified = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(FileMeta {
        len: m.len(),
        modified_ms: u64::try_from(modified.as_millis()).ok()?,
    })
}

/// Rows of `ps -A -o pid=,ppid=,state=,time=,etime=`; a line that does not parse is skipped, and a missing or bad
/// elapsed time only leaves that field empty.
pub(crate) fn parse_ps_rows(text: &str) -> Vec<ProcRow> {
    text.lines().filter_map(parse_ps_row).collect()
}

fn parse_ps_row(line: &str) -> Option<ProcRow> {
    let mut f = line.split_whitespace();
    let pid = f.next()?.parse().ok()?;
    let ppid = f.next()?.parse().ok()?;
    let state = f.next()?.chars().next()?;
    let cpu_ms = parse_cpu_time(f.next()?)?;
    let elapsed_ms = f.next().and_then(parse_elapsed);
    Some(ProcRow {
        pid,
        ppid,
        state,
        cpu_ms,
        elapsed_ms,
    })
}

impl Host for SysHost {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(0))
            .unwrap_or(0)
    }

    fn procs(&self) -> Option<Vec<ProcRow>> {
        let out = Command::new("ps")
            .args(["-A", "-o", "pid=,ppid=,state=,time=,etime="])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let rows = parse_ps_rows(&String::from_utf8(out.stdout).ok()?);
        (!rows.is_empty()).then_some(rows)
    }

    fn turn(&self, pid: u32) -> Turn {
        // only `<pid>.json`; the `<pid>.<hash>.key` files are never opened
        std::fs::read_to_string(self.registry_dir.join(format!("{pid}.json")))
            .map(|t| parse_turn(&t))
            .unwrap_or_default()
    }

    fn transcript(&self, rec: &Record) -> Option<FileMeta> {
        if !safe_component(&rec.session_id) {
            return None;
        }
        let file = format!("{}.jsonl", rec.session_id);
        let direct = self.projects_dir.join(encode_cwd(&rec.cwd)).join(&file);
        if let Some(m) = meta_of(&direct) {
            return Some(m);
        }
        // the encoding of a working directory is undocumented: look in every project directory for the name
        std::fs::read_dir(&self.projects_dir)
            .ok()?
            .flatten()
            .find_map(|d| meta_of(&d.path().join(&file)))
    }

    fn task_output(&self, rec: &Record) -> Option<FileMeta> {
        if !safe_component(&rec.session_id) {
            return None;
        }
        let dir = self
            .task_root
            .join(encode_cwd(&rec.cwd))
            .join(&rec.session_id)
            .join("tasks");
        std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .filter_map(|e| meta_of(&e.path()))
            .max_by_key(|m| m.modified_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::probe::Status;
    use std::os::unix::fs::PermissionsExt;

    fn tmp(tag: &str) -> crate::store::test_dir::TestDir {
        crate::store::test_dir::TestDir::new(&format!("host-{tag}"))
    }

    fn rec(session: &str, cwd: &str) -> Record {
        Record {
            pid: 7,
            session_id: session.into(),
            cwd: cwd.into(),
            name: None,
            proc_start: "x".into(),
            pid_domain: "darwin".into(),
        }
    }

    fn host(d: &Path) -> SysHost {
        SysHost {
            registry_dir: d.join("sessions"),
            projects_dir: d.join("projects"),
            task_root: d.join("tmp"),
        }
    }

    #[test]
    fn ps_rows_parse_and_bad_lines_are_skipped() {
        let rows = parse_ps_rows(
            "    1     0 Ss    82:51.93\n  300     1 R+     0:00.78\nbroken line\n  9 1 Z 0:00.00\n",
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].state, 'S');
        assert_eq!(rows[1].cpu_ms, 780);
        assert_eq!(rows[2].state, 'Z');
    }

    #[test]
    fn a_row_may_carry_the_elapsed_time_and_a_bad_one_only_loses_it() {
        let rows = parse_ps_rows(
            "  300     1 R+     0:00.78 2-03:04:05\n  301     1 S      0:00.10 garbage\n  302 1 S 0:00.10 05:30\n",
        );
        assert_eq!(rows.len(), 3, "a bad age never drops the row");
        assert_eq!(rows[0].elapsed_ms, Some(183_845_000));
        assert_eq!(rows[1].elapsed_ms, None);
        assert_eq!(rows[2].elapsed_ms, Some(330_000));
    }

    #[test]
    fn the_real_table_gives_this_process_an_age_that_is_not_in_the_future() {
        let rows = host(Path::new("/nonexistent"))
            .procs()
            .expect("ps works here");
        let me = rows.iter().find(|r| r.pid == std::process::id()).unwrap();
        let age = me.elapsed_ms.expect("ps reports the elapsed time here");
        assert!(age < 24 * 3_600_000, "a test process is young: {age}");
    }

    #[test]
    fn the_real_table_lists_this_process() {
        let h = host(Path::new("/nonexistent"));
        let rows = h.procs().expect("ps works here");
        let me = std::process::id();
        assert!(rows.iter().any(|r| r.pid == me));
    }

    #[test]
    fn cwd_encoding_replaces_everything_but_letters_and_digits() {
        assert_eq!(encode_cwd("/Users/a.b/x_y"), "-Users-a-b-x-y");
    }

    #[test]
    fn transcript_metadata_is_read_without_opening_the_file() {
        let d = tmp("transcript");
        let dir = d.join("projects/-w-x");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("abc-1.jsonl");
        std::fs::write(&f, "secret conversation").unwrap();
        // mode 000: the content cannot be read, so only metadata can have been used
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o000)).unwrap();
        let m = host(&d).transcript(&rec("abc-1", "/w/x")).unwrap();
        assert_eq!(m.len, 19);
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[test]
    fn a_transcript_in_an_unexpected_directory_is_still_found_by_name() {
        let d = tmp("fallback");
        std::fs::create_dir_all(d.join("projects/some-other-layout")).unwrap();
        std::fs::write(d.join("projects/some-other-layout/s9.jsonl"), "x").unwrap();
        assert!(host(&d).transcript(&rec("s9", "/w")).is_some());
    }

    #[test]
    fn a_missing_transcript_is_none() {
        let d = tmp("missing");
        assert!(host(&d).transcript(&rec("nope", "/w")).is_none());
    }

    #[test]
    fn a_session_id_that_is_not_a_plain_name_is_never_used_as_a_path() {
        let d = tmp("evil");
        std::fs::write(d.join("outside.jsonl"), "x").unwrap();
        let h = host(&d);
        assert!(h.transcript(&rec("../outside", "/w")).is_none());
        assert!(h.task_output(&rec("../..", "/w")).is_none());
    }

    #[test]
    fn task_output_is_the_newest_file_of_the_tasks_directory() {
        let d = tmp("tasks");
        let dir = d.join("tmp/-w/s1/tasks");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.output"), "1").unwrap();
        std::fs::write(dir.join("b.output"), "22").unwrap();
        assert!(host(&d).task_output(&rec("s1", "/w")).is_some());
        assert!(host(&d).task_output(&rec("s2", "/w")).is_none());
    }

    #[test]
    fn the_turn_comes_from_the_pid_file_and_a_missing_file_is_unknown() {
        let d = tmp("turn");
        std::fs::create_dir_all(d.join("sessions")).unwrap();
        std::fs::write(
            d.join("sessions/7.json"),
            r#"{"pid":7,"status":"idle","statusUpdatedAt":5}"#,
        )
        .unwrap();
        let h = host(&d);
        assert_eq!(h.turn(7).status, Some(Status::Idle));
        assert_eq!(h.turn(8), Turn::default());
    }
}
