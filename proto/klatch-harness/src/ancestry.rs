//! Walk up from a process to the harness that started it, and say which session that is.

use crate::liveness::{Liveness, check};
use crate::process::ProcessTable;
use crate::record::Registry;

/// The session a process belongs to. Same-user evidence, so it is unattributed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub session_id: String,
    pub name: Option<String>,
    pub cwd: String,
    /// The harness process the walk reached.
    pub pid: u32,
}

/// Why no session was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Miss {
    /// No ancestor has a registry record.
    NoSession,
    /// An ancestor has a record but it is not live (the reason is the nearest one). A stale record never
    /// supplies an identity.
    Stale(Liveness),
}

/// The most steps up the tree; real trees are far shallower, and this bounds a cyclic table.
const MAX_DEPTH: usize = 64;

/// Starting at `start`, visit it and each parent in turn. The nearest process that has a live registry record
/// is the session. A record that fails the liveness check is skipped and never supplies an identity.
pub fn find_session(
    start: u32,
    procs: &dyn ProcessTable,
    registry: &dyn Registry,
    domain: &str,
) -> Result<Found, Miss> {
    let mut stale = None;
    let mut seen = Vec::new();
    let mut pid = start;
    while pid > 1 && seen.len() < MAX_DEPTH && !seen.contains(&pid) {
        seen.push(pid);
        let Some(info) = procs.info(pid) else { break };
        if let Some(rec) = registry.record_for(pid) {
            match check(&rec, procs, domain) {
                Liveness::Live => {
                    return Ok(Found {
                        session_id: rec.session_id,
                        name: rec.name,
                        cwd: rec.cwd,
                        pid,
                    });
                }
                other => {
                    stale.get_or_insert(other);
                }
            }
        }
        pid = info.ppid;
    }
    Err(stale.map_or(Miss::NoSession, Miss::Stale))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::liveness::fake::FakeTable;
    use crate::record::Record;
    use std::collections::HashMap;

    const T: &str = "Fri Nov 6 03:04:05 2026";

    #[derive(Default)]
    struct FakeRegistry(HashMap<u32, Record>);
    impl FakeRegistry {
        fn with(mut self, pid: u32, session: &str, name: Option<&str>, start: &str) -> Self {
            self.0.insert(
                pid,
                Record {
                    pid,
                    session_id: session.into(),
                    cwd: "/w".into(),
                    name: name.map(Into::into),
                    proc_start: start.into(),
                    pid_domain: "darwin".into(),
                },
            );
            self
        }
    }
    impl Registry for FakeRegistry {
        fn record_for(&self, pid: u32) -> Option<Record> {
            self.0.get(&pid).cloned()
        }
    }

    // tool (30) -> shell (20) -> claude (10) -> terminal (5) -> launchd (1)
    fn tree() -> FakeTable {
        FakeTable::default()
            .with(30, 20, "Sat Nov 7 10:00:00 2026", "klatch")
            .with(20, 10, "Sat Nov 7 09:59:59 2026", "zsh")
            .with(10, 5, T, "claude")
            .with(5, 1, "Fri Nov 6 01:00:00 2026", "Terminal")
            .with(1, 0, "Thu Nov 5 00:00:00 2026", "launchd")
    }

    #[test]
    fn finds_the_session_of_the_harness_above_the_process() {
        let reg = FakeRegistry::default().with(10, "sess-A", Some("klatch-1b"), T);
        let f = find_session(30, &tree(), &reg, "darwin").unwrap();
        assert_eq!(f.session_id, "sess-A");
        assert_eq!(f.name.as_deref(), Some("klatch-1b"));
        assert_eq!(f.pid, 10);
    }

    #[test]
    fn no_record_anywhere_is_no_session() {
        assert_eq!(
            find_session(30, &tree(), &FakeRegistry::default(), "darwin"),
            Err(Miss::NoSession)
        );
    }

    #[test]
    fn a_stale_record_at_an_ancestor_pid_never_supplies_a_session() {
        // the registry still holds an old session at pid 10, but that pid was reused by a new harness process
        let reg = FakeRegistry::default().with(
            10,
            "old-session",
            Some("ghost"),
            "Mon Nov 2 08:00:00 2026",
        );
        assert_eq!(
            find_session(30, &tree(), &reg, "darwin"),
            Err(Miss::Stale(Liveness::Reused))
        );
    }

    #[test]
    fn a_record_whose_process_is_not_the_harness_is_stale() {
        let t = tree().with(10, 5, T, "bash");
        let reg = FakeRegistry::default().with(10, "s", None, T);
        assert_eq!(
            find_session(30, &t, &reg, "darwin"),
            Err(Miss::Stale(Liveness::NotHarness))
        );
    }

    #[test]
    fn a_stale_nearer_record_does_not_hide_a_live_farther_one() {
        // a nested harness: a dead record at 20, the live one at 10
        let t = tree().with(20, 10, T, "claude");
        let reg = FakeRegistry::default()
            .with(20, "dead", None, "Mon Nov 2 08:00:00 2026")
            .with(10, "live", None, T);
        assert_eq!(
            find_session(30, &t, &reg, "darwin").unwrap().session_id,
            "live"
        );
    }

    #[test]
    fn the_nearest_live_harness_wins() {
        let t = tree().with(20, 10, T, "claude");
        let reg = FakeRegistry::default()
            .with(20, "inner", None, T)
            .with(10, "outer", None, T);
        assert_eq!(
            find_session(30, &t, &reg, "darwin").unwrap().session_id,
            "inner"
        );
    }

    #[test]
    fn the_process_itself_counts_when_it_is_the_harness() {
        let reg = FakeRegistry::default().with(10, "me", None, T);
        assert_eq!(
            find_session(10, &tree(), &reg, "darwin")
                .unwrap()
                .session_id,
            "me"
        );
    }

    #[test]
    fn a_parent_loop_ends_instead_of_spinning() {
        let t = FakeTable::default().with(2, 3, T, "a").with(3, 2, T, "b");
        assert_eq!(
            find_session(2, &t, &FakeRegistry::default(), "darwin"),
            Err(Miss::NoSession)
        );
    }

    #[test]
    fn an_unknown_start_pid_is_no_session() {
        assert_eq!(
            find_session(999, &tree(), &FakeRegistry::default(), "darwin"),
            Err(Miss::NoSession)
        );
    }
}
