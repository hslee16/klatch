//! The real lease source: finds a member's session in the harness registry and probes it (#304). Same machine and
//! same user only; a session that is not in the registry (another machine, a bot, a registry that moved) is not
//! probed at all, so its claims keep their TTL.

use crate::gateway::{LeaseSource, Observed};
use crate::harness::Record;
use crate::harness::{
    FileMeta, FsRegistry, Host, ProcRow, PsTable, Registry, SysHost, Thresholds, Turn, probe,
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
            domain: if cfg!(target_os = "macos") {
                "darwin"
            } else {
                std::env::consts::OS
            },
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

impl LeaseSource for SysLeaseSource {
    fn observe(&self, _member: &str, session_id: &str, now_ms: u64) -> Option<Observed> {
        let (records, _) = self.registry.records();
        let rec = records.into_iter().find(|r| r.session_id == session_id)?;
        let host = AtTime {
            inner: &self.host,
            now_ms,
        };
        let p = probe(
            &rec,
            &PsTable,
            self.domain,
            &host,
            None,
            &Thresholds::default(),
        );
        Some(Observed {
            verdict: p.verdict,
            quiet_for_ms: p.evidence.quiet_for_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{ProcessTable, Verdict};

    fn home(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("klatch-syslease-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
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
