//! The threads that run beside one `GET /v1/stream` session (#1517, #1621): the hang-up watcher, which ends the session
//! when the client leaves, and the age timer, which cuts the connection at `--stream-max-age-secs`.
//!
//! The timer must not depend on the session ending well. A session that panics unwinds out of the scope, and the scope
//! then waits for the watcher, which is blocked reading until the client leaves: a client that stayed connected would hold
//! its member slot and a global slot with no time limit. So the way out of the session says how it ended: a normal end
//! drops the sender (the timer exits quietly, the connection is already closed), a panic sends one message first, and the
//! timer cuts the socket at once, which wakes the watcher and lets the scope finish.

use crate::loopback::Stream;
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// Read the stream for nothing but its end: a client that goes away ends the session without waiting for the next write
/// to fail.
fn watch_for_hangup(mut watch: Stream, gone: &AtomicBool) {
    let mut sink = [0u8; 256];
    while matches!(watch.read(&mut sink), Ok(n) if n > 0) {}
    gone.store(true, Ordering::SeqCst);
}

/// Held by the session's thread for as long as the session runs; when it is dropped while the thread is unwinding from a
/// panic, it asks the age timer to cut the connection now.
struct Ending(Sender<()>);

impl Drop for Ending {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let _ = self.0.send(());
        }
    }
}

/// A panic for tests of the way out of a session, in a debug build only: `KLATCH_TEST_STREAM_PANIC` set, and the test knob
/// for short limits as well. The block is compiled out of a release build, so no environment can reach it there.
#[cfg(debug_assertions)]
fn test_panic() {
    if crate::wire_ws_caps::short_limits() && std::env::var_os("KLATCH_TEST_STREAM_PANIC").is_some()
    {
        panic!("KLATCH_TEST_STREAM_PANIC: a forced panic in a stream session");
    }
}

#[cfg(not(debug_assertions))]
fn test_panic() {}

/// Run `session` with the hang-up watcher on `watch` and the age timer on `aged` beside it, and return when all three are
/// done. The timer cuts `aged` at `max_age`, and at once if the session panics.
pub(crate) fn run<R>(
    watch: Stream,
    gone: Arc<AtomicBool>,
    aged: Stream,
    max_age: Duration,
    session: impl FnOnce() -> R,
) -> R {
    let (alive, ended) = std::sync::mpsc::channel::<()>();
    std::thread::scope(|scope| {
        scope.spawn(move || watch_for_hangup(watch, &gone));
        scope.spawn(move || {
            crate::wire_ws_caps::at_max_age(&ended, max_age, || {
                let _ = aged.shutdown(std::net::Shutdown::Both);
            })
        });
        let _ending = Ending(alive);
        test_panic();
        session()
    })
}
