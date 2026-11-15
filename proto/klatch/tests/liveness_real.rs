//! Runs the liveness probe against the real Claude Code sessions of this machine and prints what it says (#310).
//! Read only: it reads the harness registry, `ps` and file metadata, never the klatch store or the Keychain.
//! Ignored by default; run it by hand:
//!   cargo test -p klatch --test liveness_real -- --ignored --nocapture
use klatch::harness::{FsRegistry, Host, PsTable, SysHost, Thresholds, list, probe};
use std::path::PathBuf;
use std::time::Instant;

#[test]
#[ignore = "reads the real harness registry; run by hand"]
fn probe_every_live_session_and_report() {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };
    let host = SysHost::standard(&home);
    let registry = FsRegistry {
        dir: host.registry_dir.clone(),
    };
    let domain = if cfg!(target_os = "macos") {
        "darwin"
    } else {
        std::env::consts::OS
    };
    let found = list(&registry, &PsTable, domain);
    println!("live sessions: {}", found.live.len());
    let t = Thresholds::default();
    let mut first = Vec::new();
    for rec in &found.live {
        let started = Instant::now();
        let p = probe(rec, &PsTable, domain, &host, None, &t);
        let cost = started.elapsed();
        println!(
            "pid {} verdict {:?} status {:?} quiet {:?}s cpu {:?} children {:?} unknown {:?} cost {:?}",
            rec.pid,
            p.verdict,
            p.evidence.turn.status,
            p.evidence.quiet_for_ms.map(|q| q / 1000),
            p.evidence.cpu_ms,
            p.evidence.children,
            p.evidence.unknown,
            cost
        );
        first.push((rec, p.sample));
    }
    std::thread::sleep(std::time::Duration::from_secs(5));
    for (rec, sample) in first {
        let p = probe(rec, &PsTable, domain, &host, Some(&sample), &t);
        println!(
            "pid {} second sample: {:?} cpu_grew {:?} now {}",
            rec.pid,
            p.verdict,
            p.evidence.cpu_grew,
            host.now_ms()
        );
    }
}
