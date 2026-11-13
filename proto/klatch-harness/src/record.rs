//! The harness registry: `<pid>.json` files, one per live session. Only the `.json` files are read; the
//! `.key` files beside them hold secrets and are never opened.

use crate::process::squash;
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
}

/// The registry directory on disk (Claude Code: `~/.claude/sessions`).
pub struct FsRegistry {
    pub dir: PathBuf,
}

impl Registry for FsRegistry {
    fn record_for(&self, pid: u32) -> Option<Record> {
        // only `<pid>.json`; the `<pid>.<hash>.key` files are never opened
        let text = std::fs::read_to_string(self.dir.join(format!("{pid}.json"))).ok()?;
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

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("klatch-reg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reads_the_record_for_a_pid() {
        let d = tmp("read");
        std::fs::write(d.join("19479.json"), GOOD).unwrap();
        let reg = FsRegistry { dir: d.clone() };
        assert_eq!(reg.record_for(19479).unwrap().session_id, "abc-123");
        assert_eq!(reg.record_for(1), None);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_record_filed_under_another_pid_is_not_believed() {
        let d = tmp("mismatch");
        std::fs::write(d.join("555.json"), GOOD).unwrap();
        assert_eq!(FsRegistry { dir: d.clone() }.record_for(555), None);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn never_opens_a_key_file() {
        let d = tmp("keys");
        std::fs::write(d.join("19479.json"), GOOD).unwrap();
        let key = d.join("19479.deadbeef.key");
        std::fs::write(&key, "secret").unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o000)).unwrap();
        // an unreadable key file proves it is not opened; the lookup still works
        assert!(FsRegistry { dir: d.clone() }.record_for(19479).is_some());
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        let _ = std::fs::remove_dir_all(d);
    }
}
