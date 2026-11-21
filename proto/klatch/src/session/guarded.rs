//! The guarded session (#147, #146). `serve` writes on the session thread, so a consumer that stops reading blocks the
//! whole session, and reads on a thread it cannot stop. `serve_guarded` puts the output behind a bounded queue with a
//! writer thread of its own: a queue that stays full for the stall limit with no progress ends the session with a
//! `stream.slow_consumer` refusal instead of stalling it, and the transport's closer unblocks the reader and the
//! writer so neither outlives the session.

use super::frames::Out;
use super::{Backend, Session, spawn_reader};
use crate::protocol::{PROTOCOL_VERSION, ServerFrame, encode_line};
use crate::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use crate::sync::mpsc::{Receiver, SyncSender, TrySendError, channel, sync_channel};
use crate::sync::thread;
use std::io::{self, BufRead, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// What a transport gives a guarded session to stop its streams: called once when the session ends, for any reason,
/// it must make a blocked read of the input and a blocked write of the output return (closing the socket does).
pub type Closer = Box<dyn FnOnce() + Send>;

/// The rule a consumer that stopped reading is refused under.
const SLOW_CONSUMER: &str = "stream.slow_consumer";

/// What the writer thread and the queue tell each other.
#[derive(Default)]
struct Shared {
    /// A write failed: the consumer is gone.
    broken: AtomicBool,
    /// The queue stayed full: the consumer is not reading.
    stalled: AtomicBool,
    /// Frames the writer has written, so the queue can tell a slow consumer from a stopped one.
    written: AtomicU64,
}

/// The session's end of the output: each write is one frame put on the queue. A full queue is waited on while the
/// writer makes progress; with none for `stall` it is a stopped consumer.
pub(super) struct QueueWriter {
    queue: SyncSender<Vec<u8>>,
    shared: Arc<Shared>,
    stall: Duration,
}

impl Write for QueueWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.shared.broken.load(Ordering::SeqCst) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        // An empty write is the health probe: it queues nothing.
        if buf.is_empty() {
            return Ok(0);
        }
        let mut frame = buf.to_vec();
        let mut progress = self.shared.written.load(Ordering::SeqCst);
        let mut since = Instant::now();
        loop {
            match self.queue.try_send(frame) {
                Ok(()) => return Ok(buf.len()),
                Err(TrySendError::Disconnected(_)) => return Err(io::ErrorKind::BrokenPipe.into()),
                Err(TrySendError::Full(back)) => frame = back,
            }
            let now = self.shared.written.load(Ordering::SeqCst);
            if now != progress {
                (progress, since) = (now, Instant::now());
            } else if since.elapsed() >= self.stall {
                self.shared.stalled.store(true, Ordering::SeqCst);
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the consumer stopped reading its stream",
                ));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The writer thread: frames from the queue to the real output, one at a time. A stalled queue is dropped and the
/// consumer, if it ever reads again, gets one refusal as the last thing.
fn write_queue<W: Write>(mut output: W, frames: Receiver<Vec<u8>>, shared: &Shared) {
    while let Ok(frame) = frames.recv() {
        if shared.stalled.load(Ordering::SeqCst) {
            break;
        }
        if output
            .write_all(&frame)
            .and_then(|()| output.flush())
            .is_err()
        {
            shared.broken.store(true, Ordering::SeqCst);
            return;
        }
        shared.written.fetch_add(1, Ordering::SeqCst);
    }
    if shared.stalled.load(Ordering::SeqCst) {
        let refusal = ServerFrame::Refusal {
            v: PROTOCOL_VERSION,
            request: None,
            rule: SLOW_CONSUMER.to_string(),
            reason: "the consumer stopped reading its stream, so the session was closed"
                .to_string(),
        };
        let _ = output
            .write_all(encode_line(&refusal).as_bytes())
            .and_then(|()| output.flush());
    }
}

/// Start the writer thread over `output`: the queue to write to, and a channel that reports when the thread is done.
fn spawn_writer<W: Write + Send + 'static>(
    output: W,
    capacity: usize,
    stall: Duration,
) -> (QueueWriter, Receiver<()>, thread::JoinHandle<()>) {
    let (queue, frames) = sync_channel(capacity.max(1));
    let (finished, done) = channel();
    let shared = Arc::new(Shared::default());
    let theirs = shared.clone();
    let handle = thread::spawn(move || {
        write_queue(output, frames, &theirs);
        let _ = finished.send(());
    });
    (
        QueueWriter {
            queue,
            shared,
            stall,
        },
        done,
        handle,
    )
}

impl<B: Backend + ?Sized> Session<'_, B> {
    /// `serve` with the output behind a bounded queue and the transport's way to stop the streams (#147, #146).
    ///
    /// A consumer that stops reading ends the session with a `TimedOut` error once the queue has been full for
    /// `SessionConfig::stall` with no progress, and is refused with `stream.slow_consumer` if it ever reads again
    /// (a closer that shuts the connection first makes that refusal unreachable, which is the point of closing).
    /// `closer` runs once on every way out; the reader thread is joined after it, so it must make a blocked read of
    /// the input return. With no closer the reader is left to end when its input does, as `serve` always did.
    /// When the input ends normally the queue is drained first, for at most `stall`.
    pub fn serve_guarded<R: BufRead + Send + 'static, W: Write + Send + 'static>(
        &self,
        input: R,
        output: W,
        closer: Option<Closer>,
    ) -> io::Result<()> {
        let (queue, drained, writer) =
            spawn_writer(output, self.config.outgoing, self.config.stall);
        let (rx, reader) = spawn_reader(input);
        let mut out = Out::probing(queue);
        let result = (if self.greet {
            self.hello(&mut out)
        } else {
            Ok(())
        })
        .and_then(|()| self.run(&rx, &mut out));
        drop(out);
        drop(rx);
        // A session that ended well waits for its last frames to be written; one that failed does not wait on a
        // consumer that may be the reason.
        let flushed = result.is_err() || drained.recv_timeout(self.config.stall).is_ok();
        let stop_streams = closer.is_some();
        if let Some(close) = closer {
            close();
        }
        if flushed && stop_streams {
            let _ = writer.join();
        }
        if stop_streams {
            let _ = reader.join();
        }
        match (result, flushed) {
            (Err(e), _) => Err(e),
            (Ok(()), false) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the consumer did not read the last frames",
            )),
            (Ok(()), true) => Ok(()),
        }
    }
}
