//! Which probe layers work on this machine, for `klatch doctor` (#311). The registry fields and the transcript layout
//! are undocumented and may change between Claude Code versions, so each layer is tried against the live sessions and
//! the answer is how many of them it could be read for. A layer that cannot be read costs the probe its evidence and
//! nothing else: unknown is never dead.

use crate::harness::probe::{Host, LAYERS, Thresholds, probe};
use crate::harness::process::ProcessTable;
use crate::harness::record::Record;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerReport {
    pub layer: &'static str,
    /// For how many of the live sessions this layer could be read.
    pub readable: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfTest {
    pub sessions: usize,
    pub layers: Vec<LayerReport>,
}

impl SelfTest {
    /// The layers that could be read for none of the sessions. Empty when there are no sessions to try. Task output
    /// is left out: it exists only while a session has background tasks, so its absence says nothing.
    pub fn broken(&self) -> Vec<&'static str> {
        self.layers
            .iter()
            .filter(|l| self.sessions > 0 && l.readable == 0 && l.layer != "task-output")
            .map(|l| l.layer)
            .collect()
    }
}

/// Probe each live session once and count, per layer, the sessions it was readable for.
pub fn self_test(
    live: &[Record],
    procs: &dyn ProcessTable,
    domain: &str,
    host: &dyn Host,
) -> SelfTest {
    let probes: Vec<_> = live
        .iter()
        .map(|r| probe(r, procs, domain, host, None, &Thresholds::default()))
        .collect();
    let layers = LAYERS
        .iter()
        .map(|layer| LayerReport {
            layer,
            readable: probes
                .iter()
                .filter(|p| !p.evidence.unknown.contains(layer))
                .count(),
        })
        .collect();
    SelfTest {
        sessions: live.len(),
        layers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::liveness::fake::FakeTable;
    use crate::harness::probe::{FileMeta, ProcRow, Status, Turn};

    const T0: &str = "Fri Nov 6 03:04:05 2026";

    struct Fake {
        turn: Turn,
        transcript: Option<FileMeta>,
        rows: Option<Vec<ProcRow>>,
    }

    impl Host for Fake {
        fn now_ms(&self) -> u64 {
            1_000_000
        }
        fn procs(&self) -> Option<Vec<ProcRow>> {
            self.rows.clone()
        }
        fn turn(&self, _: u32) -> Turn {
            self.turn
        }
        fn transcript(&self, _: &Record) -> Option<FileMeta> {
            self.transcript
        }
        fn task_output(&self, _: &Record) -> Option<FileMeta> {
            None
        }
    }

    fn rec() -> Record {
        Record {
            pid: 100,
            session_id: "s".into(),
            cwd: "/w".into(),
            name: None,
            proc_start: T0.into(),
            pid_domain: "darwin".into(),
        }
    }

    fn good() -> Fake {
        Fake {
            turn: Turn {
                status: Some(Status::Idle),
                updated_ms: Some(5),
            },
            transcript: Some(FileMeta {
                len: 1,
                modified_ms: 5,
            }),
            rows: Some(vec![ProcRow {
                pid: 100,
                ppid: 1,
                state: 'S',
                cpu_ms: 1,
            }]),
        }
    }

    fn run(h: &Fake) -> SelfTest {
        let t = FakeTable::default().with((100, 1), T0, "claude");
        self_test(&[rec()], &t, "darwin", h)
    }

    fn readable(s: &SelfTest, layer: &str) -> usize {
        s.layers.iter().find(|l| l.layer == layer).unwrap().readable
    }

    #[test]
    fn every_layer_readable_when_the_harness_looks_as_expected() {
        let s = run(&good());
        assert!(s.broken().is_empty());
        assert_eq!(readable(&s, "turn"), 1);
        assert_eq!(readable(&s, "transcript"), 1);
    }

    #[test]
    fn a_changed_registry_status_is_reported_as_a_broken_turn_layer() {
        let mut h = good();
        h.turn = Turn::default();
        assert_eq!(run(&h).broken(), vec!["turn"]);
    }

    #[test]
    fn a_moved_transcript_is_reported_as_a_broken_transcript_layer() {
        let mut h = good();
        h.transcript = None;
        assert!(run(&h).broken().contains(&"transcript"));
    }

    #[test]
    fn an_unreadable_process_table_breaks_three_layers() {
        let mut h = good();
        h.rows = None;
        let b = run(&h).broken();
        for l in ["process-state", "cpu", "children"] {
            assert!(b.contains(&l), "{l}");
        }
    }

    #[test]
    fn absent_task_output_is_not_a_broken_layer() {
        let s = run(&good());
        assert_eq!(readable(&s, "task-output"), 0);
        assert!(!s.broken().contains(&"task-output"));
    }

    #[test]
    fn no_live_session_means_nothing_is_called_broken() {
        let t = FakeTable::default();
        let s = self_test(&[], &t, "darwin", &good());
        assert_eq!(s.sessions, 0);
        assert!(s.broken().is_empty());
    }

    #[test]
    fn a_layer_that_works_for_some_sessions_is_not_broken() {
        let t = FakeTable::default()
            .with((100, 1), T0, "claude")
            .with((101, 1), T0, "claude");
        let mut second = rec();
        second.pid = 101;
        let s = self_test(&[rec(), second], &t, "darwin", &good());
        assert_eq!(readable(&s, "turn"), 2);
    }
}
