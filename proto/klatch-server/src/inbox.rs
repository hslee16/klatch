//! The input side of a session that frames are put into from outside the session's own thread (#1059; split from
//! `wire_http` for #112): `POST /v1/frames` over HTTP and the reader of a WebSocket connection both hand the session its
//! frame lines through one bounded channel, and the session reads them as it reads a pipe.

use std::io::{self, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

/// The input of a session. It ends when the connection is gone (`closed`) or the sending end is dropped.
pub(crate) struct Inbox {
    rx: Receiver<Vec<u8>>,
    closed: Arc<AtomicBool>,
    buf: Vec<u8>,
    at: usize,
}

impl Inbox {
    pub(crate) fn new(rx: Receiver<Vec<u8>>, closed: Arc<AtomicBool>) -> Inbox {
        Inbox {
            rx,
            closed,
            buf: Vec::new(),
            at: 0,
        }
    }
}

impl Read for Inbox {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.at < self.buf.len() {
                let n = out.len().min(self.buf.len() - self.at);
                out[..n].copy_from_slice(&self.buf[self.at..self.at + n]);
                self.at += n;
                return Ok(n);
            }
            if self.closed.load(Ordering::SeqCst) {
                return Ok(0);
            }
            match self.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(bytes) => {
                    self.buf = bytes;
                    self.at = 0;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(0),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::sync::mpsc::sync_channel;

    #[test]
    fn the_inbox_hands_over_posted_bytes_in_order_and_ends_when_closed_or_dropped() {
        let (tx, rx) = sync_channel(4);
        let closed = Arc::new(AtomicBool::new(false));
        let mut r = io::BufReader::new(Inbox::new(rx, closed.clone()));
        tx.send(b"a\nb".to_vec()).unwrap();
        tx.send(b"c\n".to_vec()).unwrap();
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        assert_eq!(line, "a\n");
        line.clear();
        r.read_line(&mut line).unwrap();
        assert_eq!(line, "bc\n");
        closed.store(true, Ordering::SeqCst);
        line.clear();
        assert_eq!(r.read_line(&mut line).unwrap(), 0);
        // a dropped sender ends it too
        let (tx, rx) = sync_channel::<Vec<u8>>(1);
        drop(tx);
        let mut r = Inbox::new(rx, Arc::default());
        assert_eq!(r.read(&mut [0u8; 4]).unwrap(), 0);
    }
}
