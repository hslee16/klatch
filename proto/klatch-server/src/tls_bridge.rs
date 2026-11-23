//! The pump of the mutual TLS listener (#108): after the handshake (`tls.rs`) it carries bytes between a TLS connection
//! and one end of an in-process pair of local sockets, whose other end is the HTTP handler's connection.
//!
//! Both sockets are non-blocking and the loop is level-triggered. Each direction has a bounded window ([`WINDOW`]): a
//! side that stops reading (a slow client, a handler that does not read) fills its window and the pump stops reading the
//! other side, so memory stays bounded. When the handler is done (its end of the pair is closed or shut for writing) the
//! pump closes TLS with `close_notify`, lets the client read the answer and drains what the client was still sending, so
//! closing the socket does not reset a connection whose answer has not been read. When the client is gone or breaks the
//! protocol the local pair is shut, so the handler stops. Nothing here reads, logs or keeps the content of a message.

use crate::loopback::Stream;
use rustls::ServerConnection;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

/// How much plaintext waits in either direction before the pump stops reading the other side.
pub const WINDOW: usize = 64 * 1024;

/// What a bridge has not yet passed on, and which sides are done.
#[derive(Default)]
struct Pending {
    /// Plaintext from the client, not yet taken by the handler.
    to_app: Vec<u8>,
    /// Plaintext from the handler, not yet taken by TLS.
    from_app: Vec<u8>,
    client_done: bool,
    app_done: bool,
}

impl Pending {
    /// Whether there is nothing more to carry: the client is gone, or the handler is done and all it wrote is passed on.
    fn finished(&self) -> bool {
        self.client_done || (self.app_done && self.from_app.is_empty())
    }
}

/// An error that only means "not now".
fn retry(e: &std::io::Error) -> bool {
    matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted)
}

/// One side's interest in a descriptor.
#[derive(Clone, Copy)]
struct Interest<'a> {
    stream: &'a Stream,
    read: bool,
    write: bool,
}

/// Wait for readiness of two descriptors, in one call (the one place this module polls).
fn wait(a: Interest, b: Interest) {
    let mut fds = [a, b].map(|i| libc::pollfd {
        fd: i.stream.as_raw_fd(),
        events: if i.read { libc::POLLIN } else { 0 } | if i.write { libc::POLLOUT } else { 0 },
        revents: 0,
    });
    // SAFETY: fds is a valid array of two pollfd for the duration of the call.
    unsafe {
        libc::poll(fds.as_mut_ptr(), 2, 1000);
    }
}

/// The three things a bridge holds: the TLS connection, its socket, and the local end for the handler.
struct Bridge<'a> {
    conn: ServerConnection,
    sock: &'a mut Stream,
    app: Stream,
    p: Pending,
}

/// Carry bytes between the TLS connection and the local end `app` until either side is done.
pub fn bridge(conn: ServerConnection, sock: &mut Stream, app: Stream) {
    if sock.set_nonblocking(true).is_err() || app.set_nonblocking(true).is_err() {
        return;
    }
    let mut b = Bridge {
        conn,
        sock,
        app,
        p: Pending::default(),
    };
    let mut buf = [0u8; 16 * 1024];
    while !b.p.finished() {
        if b.turn(&mut buf).is_err() {
            break;
        }
    }
    b.finish();
}

impl Bridge<'_> {
    /// One pass: write what is ready, wait, read what arrived.
    fn turn(&mut self, buf: &mut [u8]) -> Result<(), ()> {
        let Bridge { conn, sock, app, p } = self;
        flush_tls(conn, sock).map_err(|_| ())?;
        pass_to_tls(conn, p).map_err(|_| ())?;
        pass_to_app(app, p).map_err(|_| ())?;
        let want_tls = !p.client_done && p.to_app.len() < WINDOW;
        let want_app = !p.app_done && p.from_app.len() < WINDOW;
        wait(
            Interest {
                stream: sock,
                read: want_tls,
                write: conn.wants_write(),
            },
            Interest {
                stream: app,
                read: want_app,
                write: !p.to_app.is_empty(),
            },
        );
        if want_tls {
            read_tls(conn, sock, p, buf)?;
        }
        if want_app {
            read_app(app, p, buf);
        }
        Ok(())
    }

    /// End the connection. When the handler finished (its answer is whole) TLS is closed politely and the client's rest
    /// is drained; when the client is gone or broke the protocol, the alert TLS queued is sent and that is all. Either way
    /// the local pair is shut, so the handler stops.
    fn finish(mut self) {
        if self.p.app_done && !self.p.client_done {
            self.close_notify();
            drain(self.sock);
        } else {
            let _ = flush_tls(&mut self.conn, self.sock);
        }
        let _ = self.sock.shutdown(std::net::Shutdown::Both);
        let _ = self.app.shutdown(std::net::Shutdown::Both);
    }

    /// Close TLS properly: `close_notify`, flushed within two seconds, then the socket's write side.
    fn close_notify(&mut self) {
        self.conn.send_close_notify();
        let until = Instant::now() + Duration::from_secs(2);
        while self.conn.wants_write() {
            if Instant::now() >= until || flush_tls(&mut self.conn, self.sock).is_err() {
                break;
            }
            if self.conn.wants_write() {
                self.wait_writable();
            }
        }
        let _ = self.sock.shutdown(std::net::Shutdown::Write);
    }

    /// Wait until the socket can take more, or a second passes.
    fn wait_writable(&self) {
        let (sock, app) = (&*self.sock, &self.app);
        let (write, idle) = (true, false);
        wait(
            Interest {
                stream: sock,
                read: idle,
                write,
            },
            Interest {
                stream: app,
                read: idle,
                write: idle,
            },
        );
    }
}

/// Write what TLS has ready; `WouldBlock` leaves the rest for the next turn.
fn flush_tls(conn: &mut ServerConnection, sock: &mut Stream) -> std::io::Result<()> {
    while conn.wants_write() {
        match conn.write_tls(sock) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(e) if retry(&e) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Give TLS the handler's bytes it will take.
fn pass_to_tls(conn: &mut ServerConnection, p: &mut Pending) -> std::io::Result<()> {
    if p.from_app.is_empty() {
        return Ok(());
    }
    let n = conn.writer().write(&p.from_app)?;
    p.from_app.drain(..n);
    Ok(())
}

/// Give the handler the client's bytes it will take.
fn pass_to_app(app: &mut Stream, p: &mut Pending) -> std::io::Result<()> {
    if p.to_app.is_empty() {
        return Ok(());
    }
    match app.write(&p.to_app) {
        Ok(n) => p.to_app.drain(..n),
        Err(e) if retry(&e) => return Ok(()),
        Err(e) => return Err(e),
    };
    Ok(())
}

/// Take TLS records from the socket: whether the client closed the connection.
fn take_records(conn: &mut ServerConnection, sock: &mut Stream) -> Result<bool, ()> {
    match conn.read_tls(sock) {
        Ok(0) => Ok(true),
        Ok(_) => Ok(false),
        Err(e) if retry(&e) => Ok(false),
        Err(_) => Err(()),
    }
}

/// Move the plaintext TLS has decrypted to the pending bytes for the handler; the end of it is the client's end.
fn drain_plaintext(conn: &mut ServerConnection, p: &mut Pending, buf: &mut [u8]) {
    loop {
        match conn.reader().read(buf) {
            Ok(n) if n > 0 => p.to_app.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == ErrorKind::WouldBlock => return,
            // a clean end (`close_notify`), or a cut without one
            _ => {
                p.client_done = true;
                return;
            }
        }
    }
}

/// Take what the client sent: TLS records in, plaintext out. A protocol error ends the connection (rustls has queued the
/// alert; [`finish`] sends it).
fn read_tls(
    conn: &mut ServerConnection,
    sock: &mut Stream,
    p: &mut Pending,
    buf: &mut [u8],
) -> Result<(), ()> {
    p.client_done |= take_records(conn, sock)?;
    conn.process_new_packets().map_err(|_| ())?;
    drain_plaintext(conn, p, buf);
    Ok(())
}

/// Take what the handler wrote; its end of the stream is the end of the answer.
fn read_app(app: &mut Stream, p: &mut Pending, buf: &mut [u8]) {
    match app.read(buf) {
        Ok(n) if n > 0 => p.from_app.extend_from_slice(&buf[..n]),
        Err(e) if retry(&e) => {}
        _ => p.app_done = true,
    }
}

/// Read and drop what the client is still sending, for a moment and up to a bound.
fn drain(sock: &mut Stream) {
    let _ = sock.set_nonblocking(false);
    let _ = sock.set_read_timeout(Some(Duration::from_millis(200)));
    let mut sink = [0u8; 4096];
    let mut left = 256 * 1024usize;
    while left > 0 && sock.read(&mut sink).is_ok_and(|n| n > 0) {
        left = left.saturating_sub(sink.len());
    }
}
