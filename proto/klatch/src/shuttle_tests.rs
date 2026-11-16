//! Randomized-schedule tests of the in-process shared-state code (#406, DESIGN section 32 item 83). Compiled only with
//! `RUSTFLAGS="--cfg shuttle"`; run them with `proto/scripts/shuttle`. Shuttle runs the closure many times, each with a
//! different random order of the threads' steps, and fails a run when an assertion breaks or the threads deadlock. A pass
//! means no failing schedule among those tried, not that none exists. Shuttle does not model time (a timeout never
//! fires) and cannot see SQLite, `flock` or the OS, so none of that is covered here; see CONTRIBUTING.md.

use crate::sync::{mpsc, thread};
use std::collections::HashSet;
use std::io::Cursor;
use std::time::Duration;

/// Run `f` under Shuttle with a fixed seed, so a failure repeats. `SHUTTLE_SEED` and `SHUTTLE_ITERS` override.
fn explore<F: Fn() + Send + Sync + 'static>(f: F) {
    let env = |name: &str, default: u64| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    shuttle::check_random_with_seed(
        f,
        env("SHUTTLE_SEED", 406),
        env("SHUTTLE_ITERS", 2000) as usize,
    );
}

#[test]
fn message_ids_stay_unique_when_threads_race_in_one_millisecond() {
    explore(|| {
        let workers: Vec<_> = (0..3)
            .map(|_| {
                thread::spawn(|| vec![crate::store::ids::new_id(5), crate::store::ids::new_id(5)])
            })
            .collect();
        let all: Vec<String> = workers
            .into_iter()
            .flat_map(|w| w.join().unwrap())
            .collect();
        let unique: HashSet<_> = all.iter().collect();
        assert_eq!(unique.len(), all.len(), "a duplicate id: {all:?}");
    });
}

fn ack_lines(n: u64) -> Cursor<Vec<u8>> {
    let mut text = String::new();
    for seq in 1..=n {
        text.push_str(&format!(r#"{{"type":"ack","v":1,"room":"r","seq":{seq}}}"#));
        text.push('\n');
    }
    Cursor::new(text.into_bytes())
}

#[test]
fn the_session_reader_delivers_every_frame_once_in_order_then_reports_the_input_closed() {
    explore(|| {
        let rx = crate::session::read_in_background(ack_lines(4));
        let mut seqs = Vec::new();
        let mut open = true;
        let mut rounds = 0;
        while open {
            for item in crate::session::collect(&rx, Duration::from_millis(1), &mut open) {
                match item {
                    Ok(crate::protocol::ClientFrame::Ack { seq, .. }) => seqs.push(seq),
                    other => panic!("unexpected item {other:?}"),
                }
            }
            rounds += 1;
            assert!(rounds < 20, "the input never reported closed");
        }
        assert_eq!(seqs, vec![1, 2, 3, 4]);
    });
}

#[test]
fn the_client_reader_sends_every_frame_in_order_and_the_end_marker_last() {
    explore(|| {
        let mut text = String::new();
        for ts in 1..=3 {
            text.push_str(&crate::protocol::encode_line(
                &crate::protocol::ServerFrame::Heartbeat { v: 1, ts },
            ));
        }
        let (tx, rx) = mpsc::channel();
        let reader =
            thread::spawn(move || crate::client::pump_frames(Cursor::new(text.into_bytes()), tx));
        let mut stamps = Vec::new();
        loop {
            match rx
                .recv()
                .expect("the end marker comes before the channel closes")
            {
                Some(Ok(crate::protocol::ServerFrame::Heartbeat { ts, .. })) => stamps.push(ts),
                Some(other) => panic!("unexpected {other:?}"),
                None => break,
            }
        }
        reader.join().unwrap();
        assert_eq!(stamps, vec![1, 2, 3]);
        assert!(rx.recv().is_err(), "nothing follows the end marker");
    });
}

#[test]
fn within_returns_the_probes_answer_when_the_probe_finishes() {
    // Shuttle never fires a timeout, so this covers the hand-off between the probe thread and the waiter, not the
    // time limit itself.
    explore(|| {
        let other = thread::spawn(|| crate::sync::within(Duration::from_secs(1), || 8));
        let mine = crate::sync::within(Duration::from_secs(1), || 7);
        assert_eq!(mine, Some(7));
        assert_eq!(other.join().unwrap(), Some(8));
    });
}
