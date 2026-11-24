//! Randomized-schedule tests of the in-process shared-state code (#406, DESIGN section 32 item 83). Compiled only with
//! `RUSTFLAGS="--cfg shuttle"`; run them with `proto/scripts/shuttle`. Shuttle runs the closure many times, each with a
//! different random order of the threads' steps, and fails a run when an assertion breaks or the threads deadlock. A pass
//! means no failing schedule among those tried, not that none exists. Shuttle has no clock (a timeout never fires), so
//! `sync::within` stands a timer thread in for its limit under Shuttle (#666, DESIGN item 102): the limit can pass at any
//! step, which the tests below use; `sync::recv_within` does the same for the session's poll period and heartbeat (the
//! `time` module). How long any limit is, the wake and wait polling and the SQLite busy retries are not covered. Shuttle also cannot see SQLite, `flock` or the OS; see CONTRIBUTING.md.

use crate::sync::{mpsc, thread};
use std::collections::HashSet;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering as StdOrdering};
use std::time::Duration;

mod time;

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
fn within_hands_each_waiter_its_own_probes_answer_or_none_never_the_others() {
    // Two waiters at once. Under Shuttle the limit can pass at any step (#666), so each gets its own probe's answer or
    // None; what it must never get is the other probe's answer.
    explore(|| {
        let other = thread::spawn(|| crate::sync::within(Duration::from_secs(1), || 8));
        let mine = crate::sync::within(Duration::from_secs(1), || 7);
        assert!(matches!(mine, Some(7) | None), "got {mine:?}");
        let theirs = other.join().unwrap();
        assert!(matches!(theirs, Some(8) | None), "got {theirs:?}");
    });
}

#[test]
fn within_gives_up_on_a_probe_that_never_answers_in_every_schedule() {
    // The probe waits on a channel nobody sends on. Shuttle has no clock, so under Shuttle `within` stands a timer
    // thread in for the time limit (#666): whichever step comes first, the waiter must come back with None and not
    // deadlock.
    explore(|| {
        let (_keep_open, never) = mpsc::channel::<()>();
        let got = crate::sync::within(Duration::from_secs(1), move || never.recv().ok());
        assert_eq!(got, None);
    });
}

#[test]
fn within_answers_the_probes_value_or_none_and_a_late_answer_is_harmless() {
    // The probe and the timer race. The result is the probe's value or None; when the timer wins, the probe's answer
    // arrives after the waiter has gone, and sending it must not panic the probe thread. Both outcomes must occur
    // among the schedules, or the timer path was never explored.
    let (answered, gave_up) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let (a, g) = (answered.clone(), gave_up.clone());
    explore(move || {
        match crate::sync::within(Duration::from_secs(1), || 41 + 1) {
            Some(v) => {
                assert_eq!(v, 42);
                a.fetch_add(1, StdOrdering::Relaxed);
            }
            None => {
                g.fetch_add(1, StdOrdering::Relaxed);
            }
        }
        // let the abandoned probe and timer threads run to their ends inside this execution
        for _ in 0..4 {
            thread::yield_now();
        }
    });
    assert!(
        answered.load(StdOrdering::Relaxed) > 0,
        "the probe never won"
    );
    assert!(
        gave_up.load(StdOrdering::Relaxed) > 0,
        "the timer never won"
    );
}

#[test]
fn the_session_reader_ends_quietly_when_the_session_stops_listening_mid_stream() {
    // The session drops its end after the first frame (it is shutting down, #146). The reader thread has more frames to
    // send; a failed send must end it, not panic it.
    explore(|| {
        let (rx, reader) = crate::session::spawn_reader(ack_lines(4));
        let first = rx.recv().expect("the first frame arrives");
        assert!(first.is_ok());
        drop(rx);
        reader
            .join()
            .expect("the reader thread panicked when nobody was listening");
    });
}

#[test]
fn the_client_reader_ends_quietly_when_the_receiver_is_gone() {
    // The client dropped its receiver (the connection is closing) while the reader still has frames to send.
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
        let first = rx.recv().expect("the first frame arrives");
        assert!(matches!(first, Some(Ok(_))));
        drop(rx);
        reader
            .join()
            .expect("the reader thread panicked when nobody was listening");
    });
}

fn acp_lines(lines: &[serde_json::Value]) -> Cursor<Vec<u8>> {
    Cursor::new(
        lines
            .iter()
            .map(|l| format!("{l}\n"))
            .collect::<String>()
            .into_bytes(),
    )
}

fn acp_prompt(id: u64, session: &str) -> serde_json::Value {
    serde_json::json!({"jsonrpc":"2.0","id":id,"method":"session/prompt","params":{"sessionId":session,"prompt":[]}})
}

fn acp_cancel(session: &str) -> serde_json::Value {
    serde_json::json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session}})
}

#[test]
fn the_acp_reader_raises_only_the_flag_of_the_named_session_before_it_reports_the_input_closed() {
    // The handler takes lines while the reader is still reading. Whatever the order of their steps, by the time the
    // channel reports closed the cancel has been read: the named session's flag is up, the other's is not, the cancel
    // itself is not passed on, and the prompts arrive in the order they were sent.
    explore(|| {
        let input = acp_lines(&[
            acp_prompt(1, "a"),
            acp_prompt(2, "b"),
            acp_cancel("a"),
            acp_prompt(3, "a"),
        ]);
        let (rx, reader) = crate::session::acp_reader::spawn(input);
        let mut got = Vec::new();
        while let Ok(item) = rx.recv() {
            got.push(item.expect("no read error"));
        }
        reader.join().unwrap();
        let ids: Vec<u64> = got
            .iter()
            .map(|(line, _)| {
                serde_json::from_str::<serde_json::Value>(line).unwrap()["id"]
                    .as_u64()
                    .unwrap()
            })
            .collect();
        assert_eq!(ids, vec![1, 2, 3], "order, and no cancel passed on");
        let up = |n: usize| got[n].1.as_ref().unwrap().load(StdOrdering::SeqCst);
        assert!(up(0), "the cancelled prompt's flag is up");
        assert!(!up(1), "another session's flag is not");
        assert!(!up(2), "a prompt sent after the cancel is not cancelled");
    });
}

#[test]
fn the_acp_reader_ends_quietly_when_the_handler_stops_listening_mid_stream() {
    // The handler drops its end after the first line (a write failed). The reader has more to pass on; a failed send
    // must end it, not panic it or leave it waiting for room.
    explore(|| {
        let input = acp_lines(&[acp_prompt(1, "a"), acp_prompt(2, "a"), acp_prompt(3, "a")]);
        let (rx, reader) = crate::session::acp_reader::spawn(input);
        assert!(rx.recv().unwrap().is_ok());
        drop(rx);
        reader
            .join()
            .expect("the reader panicked when nobody was listening");
    });
}
