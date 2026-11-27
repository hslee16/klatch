//! The real lease source: finds a member's session in the harness registry and probes it (#304). Same machine and
//! same user only; a session that is not in the registry (another machine, a bot, a registry that moved) is not
//! probed at all, so its claims keep their TTL.

use crate::gateway::{LeaseSource, Observed};
use crate::harness::Record;
use crate::harness::{
    FileMeta, FsRegistry, Host, ProcRow, ProcessTable, PsTable, SysHost, Thresholds, Turn, Verdict,
    probe,
};
use std::path::Path;

pub struct SysLeaseSource {
    registry: FsRegistry,
    host: SysHost,
    domain: &'static str,
}

impl SysLeaseSource {
    /// The registry and files under a home directory.
    pub fn under_home(home: &Path) -> SysLeaseSource {
        let host = SysHost::standard(home);
        SysLeaseSource {
            registry: FsRegistry {
                dir: host.registry_dir.clone(),
            },
            host,
            domain: crate::harness::this_pid_domain(),
        }
    }
}

/// The real host with the clock the gateway runs on, so a controlled clock moves file ages too.
struct AtTime<'a> {
    inner: &'a SysHost,
    now_ms: u64,
}

impl Host for AtTime<'_> {
    fn now_ms(&self) -> u64 {
        self.now_ms
    }
    fn procs(&self) -> Option<Vec<ProcRow>> {
        self.inner.procs()
    }
    fn turn(&self, pid: u32) -> Turn {
        self.inner.turn(pid)
    }
    fn transcript(&self, rec: &Record) -> Option<FileMeta> {
        self.inner.transcript(rec)
    }
    fn task_output(&self, rec: &Record) -> Option<FileMeta> {
        self.inner.task_output(rec)
    }
}

impl SysLeaseSource {
    /// The pids in a readable process table; `None` when it cannot be read (the listing is then ranked by age alone).
    fn alive(&self) -> Option<std::collections::HashSet<u32>> {
        Some(self.host.procs()?.iter().map(|p| p.pid).collect())
    }
}

impl LeaseSource for SysLeaseSource {
    fn observe(&self, _member: &str, session_id: &str, now_ms: u64) -> Option<Observed> {
        let host = AtTime {
            inner: &self.host,
            now_ms,
        };
        observe_in(
            &self.registry,
            &PsTable,
            &host,
            self.domain,
            session_id,
            &self.alive(),
        )
    }
}

/// What the registry, the process table and the host say about `session_id`, as the lease rules need it (#1706).
///
/// Every record that claims the session id is probed, because a same-user process can write a record carrying any
/// session id under any pid (a low live pid to read as gone and shorten the victim's claims, or a live process named
/// like the harness to read as alive and keep a dead member's claims from ever lapsing). One record: it is the
/// session's. More than one: if none of them is the live harness the answer is `Gone`, which is true whichever of them
/// is the real one; otherwise the records disagree about who the session is and nothing here can tell the real one from
/// the forged, so the answer is `None` and the lease is left alone (neither extended nor shortened; its TTL decides).
/// A harness restarted under a new pid with the stale file of the old one left behind lands here too, and lapses by TTL.
///
/// A listing a bound cut (more records than the scan reads, a flood of fresher ones) is an incomplete view: the record
/// found may be a forged one with the real one cut out, and what is absent may be only unread. The answer is then `None`
/// whatever was found, never `Gone` (which shortens claims) and never alive (which extends them); an unreadable process
/// table is not affected (it leaves the order by age and fails closed as before).
fn observe_in(
    registry: &FsRegistry,
    procs: &dyn ProcessTable,
    host: &dyn Host,
    domain: &str,
    session_id: &str,
    alive: &Option<std::collections::HashSet<u32>>,
) -> Option<Observed> {
    // one read of the process table for every record (a lookup on the real one runs `ps -A`, #1569)
    let pinned = procs.pin();
    let procs: &dyn ProcessTable = pinned.as_deref().unwrap_or(procs);
    let (records, _, truncated) = registry.records_alive(alive.as_ref());
    if truncated {
        return None;
    }
    let claims: Vec<Record> = records
        .into_iter()
        .filter(|r| r.session_id == session_id)
        .collect();
    let seen: Vec<Observed> = claims
        .iter()
        .map(|rec| {
            let p = probe(rec, procs, domain, host, None, &Thresholds::default());
            Observed {
                verdict: p.verdict,
                quiet_for_ms: p.evidence.quiet_for_ms,
            }
        })
        .collect();
    match seen.as_slice() {
        [] => None,
        [one] => Some(*one),
        many if many.iter().all(|o| o.verdict == Verdict::Gone) => Some(many[0]),
        _ => None,
    }
}

#[cfg(test)]
mod at_time_tests;

#[cfg(test)]
mod observe_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{ProcessTable, Verdict};

    fn home(tag: &str) -> crate::store::test_dir::TestDir {
        let d = crate::store::test_dir::TestDir::new(&format!("syslease-{tag}"));
        std::fs::create_dir_all(d.join(".claude/sessions")).unwrap();
        d
    }

    fn record_for_this_process(home: &Path, session: &str) {
        let pid = std::process::id();
        let start = PsTable.info(pid).expect("ps sees this process").start_utc;
        let domain = if cfg!(target_os = "macos") {
            "darwin"
        } else {
            std::env::consts::OS
        };
        let json = format!(
            r#"{{"pid":{pid},"sessionId":"{session}","cwd":"/w","procStart":"{start}","pidDomain":"{domain}","status":"busy","statusUpdatedAt":1}}"#
        );
        std::fs::write(home.join(format!(".claude/sessions/{pid}.json")), json).unwrap();
    }

    #[test]
    fn a_session_not_in_the_registry_is_not_probed() {
        let h = home("absent");
        assert!(
            SysLeaseSource::under_home(&h)
                .observe("m", "nope", 5_000)
                .is_none()
        );
    }

    #[test]
    fn a_registered_process_that_is_not_the_harness_reads_as_gone() {
        // the real path end to end: registry file, ps identity (this test binary is not named claude), verdict
        let h = home("gone");
        record_for_this_process(&h, "sess-1");
        let seen = SysLeaseSource::under_home(&h)
            .observe("m", "sess-1", 5_000)
            .unwrap();
        assert_eq!(seen.verdict, Verdict::Gone);
    }
}

#[cfg(test)]
mod forged_tests {
    use super::*;
    use crate::harness::ProcessTable;

    #[test]
    fn forged_records_for_pids_that_do_not_run_do_not_hide_a_running_session() {
        let h = crate::store::test_dir::TestDir::new("syslease-forged");
        let dir = h.join(".claude/sessions");
        std::fs::create_dir_all(&dir).unwrap();
        let pid = std::process::id();
        let start = crate::harness::PsTable.info(pid).unwrap().start_utc;
        let rec = |pid: u32, session: &str, start: &str| {
            format!(
                r#"{{"pid":{pid},"sessionId":"{session}","cwd":"/w","procStart":"{start}","pidDomain":"x"}}"#
            )
        };
        let real = dir.join(format!("{pid}.json"));
        std::fs::write(&real, rec(pid, "real", &start)).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&real)
            .unwrap()
            .set_modified(
                std::time::SystemTime::now() - std::time::Duration::from_secs(30 * 86_400),
            )
            .unwrap();
        for forged in 4_000_000..4_000_000 + 300u32 {
            std::fs::write(dir.join(format!("{forged}.json")), rec(forged, "fake", "t")).unwrap();
        }
        let seen = SysLeaseSource::under_home(&h).observe("m", "real", 5_000);
        assert!(seen.is_some(), "the running session was hidden");
    }
}
