//! Walk up from a process to the harness that started it, and say which session that is.

use crate::harness::liveness::{Liveness, check};
use crate::harness::process::{Lookup, ProcessTable};
use crate::harness::record::Registry;

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
    /// The process table could not be read, so the walk could not look. Not the same as no session (#525).
    TableUnreadable,
}

/// The most steps up the tree; real trees are far shallower, and this bounds a cyclic table.
const MAX_DEPTH: usize = 64;

/// Whether the walk goes on to `pid`: not the init process, not too deep, and not a process already visited.
fn worth_visiting(pid: u32, seen: &[u32]) -> bool {
    pid > 1 && seen.len() < MAX_DEPTH && !seen.contains(&pid)
}

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
    while worth_visiting(pid, &seen) {
        seen.push(pid);
        let info = match procs.lookup(pid) {
            Lookup::Found(info) => info,
            Lookup::Missing => break,
            Lookup::Unreadable => return Err(Miss::TableUnreadable),
        };
        match session_at(pid, procs, registry, domain) {
            Some(Ok(found)) => return Ok(found),
            Some(Err(why)) => {
                stale.get_or_insert(why);
            }
            None => {}
        }
        pid = info.ppid;
    }
    Err(stale.map_or(Miss::NoSession, Miss::Stale))
}

/// The session whose harness is `pid`: `None` with no registry record, the session when the record passes the
/// liveness check, and otherwise why the record is stale (it supplies nothing).
fn session_at(
    pid: u32,
    procs: &dyn ProcessTable,
    registry: &dyn Registry,
    domain: &str,
) -> Option<Result<Found, Liveness>> {
    let rec = registry.record_for(pid)?;
    Some(match check(&rec, procs, domain) {
        Liveness::Live => Ok(Found {
            session_id: rec.session_id,
            name: rec.name,
            cwd: rec.cwd,
            pid,
        }),
        other => Err(other),
    })
}

/// A coding-agent process that stands between `start` and the session `found` and has no live record of its own: a
/// child (a subagent) of that session. The walk would give it the session's member, so two children of one session
/// would share one name, one cursor and one author (#503). Returns the child's pid. A nested session that has a
/// record of its own is found first by `find_session` and is never reported here: it already is a member of its own.
pub fn unregistered_agent_below(
    start: u32,
    found: &Found,
    procs: &dyn ProcessTable,
) -> Option<u32> {
    let mut seen = Vec::new();
    let mut pid = start;
    while pid != found.pid && worth_visiting(pid, &seen) {
        seen.push(pid);
        let info = procs.info(pid)?;
        if crate::harness::liveness::is_claude_code(&info.command) {
            return Some(pid);
        }
        pid = info.ppid;
    }
    None
}

/// The session above `session`: the nearest live harness record above the process that session was found at. A
/// headless worker started from an interactive session has its own record and, above it, the session that started it.
pub fn find_parent(
    session: &Found,
    procs: &dyn ProcessTable,
    registry: &dyn Registry,
    domain: &str,
) -> Option<Found> {
    let above = procs.info(session.pid)?.ppid;
    find_session(above, procs, registry, domain).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::liveness::fake::FakeTable;
    use crate::harness::process::Lookup;
    use crate::harness::record::Record;
    use std::collections::HashMap;

    const T: &str = "Fri Nov 6 03:04:05 2026";

    #[derive(Default)]
    struct FakeRegistry(HashMap<u32, Record>);
    impl FakeRegistry {
        fn with(mut self, (pid, session): (u32, &str), name: Option<&str>, start: &str) -> Self {
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
            .with((30, 20), "Sat Nov 7 10:00:00 2026", "klatch")
            .with((20, 10), "Sat Nov 7 09:59:59 2026", "zsh")
            .with((10, 5), T, "claude")
            .with((5, 1), "Fri Nov 6 01:00:00 2026", "Terminal")
            .with((1, 0), "Thu Nov 5 00:00:00 2026", "launchd")
    }

    #[test]
    fn a_claude_process_with_no_record_between_the_caller_and_the_session_is_a_child() {
        // tool (30) -> shell (20) -> claude (15, no record) -> shell (12) -> claude (10, the session)
        let t = tree()
            .with((20, 15), "Sat Nov 7 09:59:59 2026", "zsh")
            .with((15, 12), "Sat Nov 7 09:59:58 2026", "claude")
            .with((12, 10), "Sat Nov 7 09:59:57 2026", "zsh");
        let reg = FakeRegistry::default().with((10, "sess-A"), Some("klatch-1b"), T);
        let f = find_session(30, &t, &reg, "darwin").unwrap();
        assert_eq!(unregistered_agent_below(30, &f, &t), Some(15));
    }

    #[test]
    fn a_top_level_session_has_no_child_below_it() {
        let reg = FakeRegistry::default().with((10, "sess-A"), None, T);
        let f = find_session(30, &tree(), &reg, "darwin").unwrap();
        assert_eq!(unregistered_agent_below(30, &f, &tree()), None);
    }

    #[test]
    fn a_nested_session_with_a_record_of_its_own_is_not_a_child() {
        // the nearer claude (20) has its own live record, so the walk stops there and it is its own member
        let t = tree().with((20, 10), T, "claude");
        let reg = FakeRegistry::default()
            .with((20, "inner"), None, T)
            .with((10, "outer"), None, T);
        let f = find_session(30, &t, &reg, "darwin").unwrap();
        assert_eq!(f.session_id, "inner");
        assert_eq!(unregistered_agent_below(30, &f, &t), None);
    }

    #[test]
    fn finds_the_session_of_the_harness_above_the_process() {
        let reg = FakeRegistry::default().with((10, "sess-A"), Some("klatch-1b"), T);
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
            (10, "old-session"),
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
        let t = tree().with((10, 5), T, "bash");
        let reg = FakeRegistry::default().with((10, "s"), None, T);
        assert_eq!(
            find_session(30, &t, &reg, "darwin"),
            Err(Miss::Stale(Liveness::NotHarness))
        );
    }

    #[test]
    fn a_stale_nearer_record_does_not_hide_a_live_farther_one() {
        // a nested harness: a dead record at 20, the live one at 10
        let t = tree().with((20, 10), T, "claude");
        let reg = FakeRegistry::default()
            .with((20, "dead"), None, "Mon Nov 2 08:00:00 2026")
            .with((10, "live"), None, T);
        assert_eq!(
            find_session(30, &t, &reg, "darwin").unwrap().session_id,
            "live"
        );
    }

    #[test]
    fn the_nearest_live_harness_wins() {
        let t = tree().with((20, 10), T, "claude");
        let reg = FakeRegistry::default()
            .with((20, "inner"), None, T)
            .with((10, "outer"), None, T);
        assert_eq!(
            find_session(30, &t, &reg, "darwin").unwrap().session_id,
            "inner"
        );
    }

    #[test]
    fn the_session_above_a_nested_session_is_its_parent() {
        let t = tree().with((20, 10), T, "claude");
        let reg = FakeRegistry::default()
            .with((20, "inner"), Some("child"), T)
            .with((10, "outer"), Some("lead"), T);
        let inner = find_session(30, &t, &reg, "darwin").unwrap();
        let parent = find_parent(&inner, &t, &reg, "darwin").unwrap();
        assert_eq!((parent.session_id.as_str(), parent.pid), ("outer", 10));
    }

    #[test]
    fn a_session_with_no_session_above_it_has_no_parent() {
        let reg = FakeRegistry::default().with((10, "only"), None, T);
        let only = find_session(30, &tree(), &reg, "darwin").unwrap();
        assert!(find_parent(&only, &tree(), &reg, "darwin").is_none());
    }

    #[test]
    fn a_stale_record_above_a_session_is_not_its_parent() {
        let t = tree().with((20, 10), T, "claude");
        let reg = FakeRegistry::default().with((20, "inner"), None, T).with(
            (10, "outer"),
            None,
            "Mon Jan 1 00:00:00 2001",
        );
        let inner = find_session(30, &t, &reg, "darwin").unwrap();
        assert!(find_parent(&inner, &t, &reg, "darwin").is_none());
    }

    #[test]
    fn the_process_itself_counts_when_it_is_the_harness() {
        let reg = FakeRegistry::default().with((10, "me"), None, T);
        assert_eq!(
            find_session(10, &tree(), &reg, "darwin")
                .unwrap()
                .session_id,
            "me"
        );
    }

    #[test]
    fn a_parent_loop_ends_instead_of_spinning() {
        let t = FakeTable::default()
            .with((2, 3), T, "a")
            .with((3, 2), T, "b");
        assert_eq!(
            find_session(2, &t, &FakeRegistry::default(), "darwin"),
            Err(Miss::NoSession)
        );
    }

    #[test]
    fn a_process_table_that_cannot_be_read_names_itself_and_never_supplies_a_session() {
        // #525: a ps that cannot run is not "no such process", and it must not identify anyone either
        let reg = FakeRegistry::default().with((10, "s"), None, T);
        assert_eq!(
            find_session(
                30,
                &crate::harness::liveness::fake::UnreadableTable,
                &reg,
                "darwin"
            ),
            Err(Miss::TableUnreadable)
        );
    }

    #[test]
    fn a_table_that_becomes_unreadable_partway_up_is_not_a_session() {
        // the walk reaches pid 20 and then loses the table: no identity from the part already seen
        struct Flaky(FakeTable);
        impl ProcessTable for Flaky {
            fn lookup(&self, pid: u32) -> Lookup {
                if pid == 10 {
                    Lookup::Unreadable
                } else {
                    self.0.lookup(pid)
                }
            }
        }
        let reg = FakeRegistry::default().with((10, "s"), None, T);
        assert_eq!(
            find_session(30, &Flaky(tree()), &reg, "darwin"),
            Err(Miss::TableUnreadable)
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
