//! The live sessions in a harness registry (`klatch sessions`, #131): every record that parses, checked for liveness the
//! same way `find_session` checks one (RESEARCH-NOTES section 17). Read only: it lists `<pid>.json` files and never
//! opens the `.key` files beside them.

use crate::harness::liveness::{Liveness, check};
use crate::harness::process::ProcessTable;
use crate::harness::record::{Record, Registry};

/// What was found: the sessions that are running, the records that are not about one (with why), and how many files
/// looked like records but did not parse.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Sessions {
    pub live: Vec<Record>,
    pub ignored: Vec<(Record, Liveness)>,
    pub malformed: usize,
}

/// Sort every record of the registry into live and not live.
pub fn list(registry: &dyn Registry, procs: &dyn ProcessTable, domain: &str) -> Sessions {
    let (records, malformed) = registry.records();
    let mut found = Sessions {
        malformed,
        ..Sessions::default()
    };
    for rec in records {
        match check(&rec, procs, domain) {
            Liveness::Live => found.live.push(rec),
            other => found.ignored.push((rec, other)),
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::liveness::fake::FakeTable;
    use crate::harness::record::FsRegistry;
    use std::path::Path;

    const T: &str = "Fri Nov 6 03:04:05 2026";

    fn record(pid: u32, start: &str, domain: &str, extra: &str) -> String {
        format!(
            r#"{{"pid":{pid},"sessionId":"sess-{pid}","cwd":"/work/{pid}","name":"n{pid}","procStart":"{start}","pidDomain":"{domain}"{extra}}}"#
        )
    }

    fn dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "klatch-sessions-{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("t")
                .replace("::", "-")
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn put(d: &Path, name: &str, text: &str) {
        std::fs::write(d.join(name), text).unwrap();
    }

    fn table() -> FakeTable {
        FakeTable::default()
            .with((10, 1), T, "/usr/local/bin/claude") // live
            .with((20, 1), "Mon Nov 2 08:00:00 2026", "/usr/local/bin/claude") // reused: started at another time
            .with((30, 1), T, "/bin/zsh") // not the harness
            .with((60, 1), T, "claude") // live, record is old
    }

    fn found(d: &Path) -> Sessions {
        list(
            &FsRegistry {
                dir: d.to_path_buf(),
            },
            &table(),
            "darwin",
        )
    }

    #[test]
    fn live_dead_reused_wrong_program_and_other_machine_records_are_told_apart() {
        let d = dir();
        put(&d, "10.json", &record(10, T, "darwin", ""));
        put(&d, "20.json", &record(20, T, "darwin", ""));
        put(&d, "30.json", &record(30, T, "darwin", ""));
        put(&d, "40.json", &record(40, T, "darwin", "")); // no such process
        put(&d, "50.json", &record(50, T, "linux", "")); // another pid space
        let f = found(&d);
        assert_eq!(f.live.iter().map(|r| r.pid).collect::<Vec<_>>(), vec![10]);
        let why = |pid: u32| {
            f.ignored
                .iter()
                .find(|(r, _)| r.pid == pid)
                .map(|(_, l)| *l)
        };
        assert_eq!(why(20), Some(Liveness::Reused));
        assert_eq!(why(30), Some(Liveness::NotHarness));
        assert_eq!(why(40), Some(Liveness::Dead));
        assert_eq!(why(50), Some(Liveness::OtherDomain));
        assert_eq!(f.malformed, 0);
    }

    #[test]
    fn an_old_updated_at_on_a_live_session_is_still_listed() {
        let d = dir();
        put(
            &d,
            "60.json",
            &record(60, T, "darwin", r#","updatedAt":1000000"#),
        );
        assert_eq!(
            found(&d).live.iter().map(|r| r.pid).collect::<Vec<_>>(),
            vec![60]
        );
    }

    #[test]
    fn a_malformed_record_is_counted_and_never_stops_the_listing() {
        let d = dir();
        put(&d, "10.json", &record(10, T, "darwin", ""));
        put(&d, "11.json", "{ not json");
        put(&d, "12.json", r#"{"pid":12}"#); // a record missing the fields a check needs
        let f = found(&d);
        assert_eq!(f.live.len(), 1);
        assert_eq!(f.malformed, 2);
    }

    #[test]
    fn a_record_with_an_unknown_field_still_parses() {
        let d = dir();
        put(
            &d,
            "10.json",
            &record(10, T, "darwin", r#","somethingNew":{"a":1}"#),
        );
        assert_eq!(found(&d).live.len(), 1);
    }

    #[test]
    fn only_files_named_pid_dot_json_are_read_so_the_key_files_are_never_opened() {
        use crate::harness::record::is_record_file;
        assert!(is_record_file("10.json") && is_record_file("123456.json"));
        for not in [
            "10.abcdef.key",
            "10.key",
            "10.json.bak",
            "x.json",
            ".json",
            "10.JSON",
            "10",
            "notes.txt",
            "10.json.key",
        ] {
            assert!(!is_record_file(not), "{not} is not a record file");
        }
        // and a key file beside a record, holding something that must never appear, is not listed or read
        let d = dir();
        put(&d, "10.json", &record(10, T, "darwin", ""));
        put(&d, "10.deadbeef.key", "TOP-SECRET-TOKEN");
        let f = found(&d);
        assert!(format!("{f:?}").find("TOP-SECRET").is_none());
        assert_eq!(f.live.len(), 1);
    }

    #[test]
    fn a_missing_registry_directory_is_an_empty_listing() {
        let f = list(
            &FsRegistry {
                dir: "/nonexistent/klatch/registry".into(),
            },
            &table(),
            "darwin",
        );
        assert_eq!(f, Sessions::default());
    }

    #[test]
    fn the_listing_is_ordered_by_pid() {
        let d = dir();
        put(&d, "60.json", &record(60, T, "darwin", ""));
        put(&d, "10.json", &record(10, T, "darwin", ""));
        assert_eq!(
            found(&d).live.iter().map(|r| r.pid).collect::<Vec<_>>(),
            vec![10, 60]
        );
    }
}
