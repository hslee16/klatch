//! The pump of the mutual TLS listener (#108): after the handshake (`tls.rs`) it carries bytes between a TLS connection
//! and one end of an in-process pair of local sockets, whose other end is the HTTP handler's connection.
//!
//! Both sockets are non-blocking and the loop is level-triggered. Each direction has a bounded window ([`WINDOW`]): a
//! side that stops reading (a slow client, a handler that does not read) fills its window and the pump stops reading the
//! other side, so memory stays bounded.
//!
//! **Half-close.** A client that is done sending (a TCP FIN, or a TLS `close_notify`) is not a client that is gone: the
//! pump hands the handler every byte the client sent, then shuts the handler's end for writing (the handler sees the end
//! of its input, as a loopback client's FIN shows it), and goes on carrying the handler's answer until the handler is
//! done. Only then is TLS closed with `close_notify`. A client that is truly gone shows as a write or read error, and
//! the pair is shut.
//!
//! **Stall deadline.** If no byte moves on the CLIENT SOCKET in either direction (bytes written to it, bytes read from
//! it) for the stall time, the connection is closed. Bytes a handler hands to the pump do not count, so a client that
//! accepts nothing is not kept alive by the handler's own output. What this cannot see: a client that does not read but
//! whose kernel still acknowledges data (its receive window and our send buffers take a heartbeat every 15 seconds for
//! days before they are full: tens of bytes every 15 seconds) counts as moving, because the bytes did move. Cutting such a client early would need an
//! application-level acknowledgement, which the handler's protocol does not have (no test of it exists); a client that stops reading a busy
//! stream is cut once the buffers fill. A quiet event stream is not idle: it carries a heartbeat. The
//! poll never watches a descriptor nobody asked about, because a hung-up descriptor is reported even with no events
//! requested and would make the loop spin. Nothing here reads, logs or keeps the content of a message.

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
    /// The client will send nothing more (FIN or `close_notify`); it may still read.
    client_done: bool,
    /// The handler's end of the pair has been shut for writing, after all the client's bytes were given to it.
    app_eof_sent: bool,
    /// The handler is done writing.
    app_done: bool,
    /// Bytes moved on the client socket in either direction so far: the stall deadline's clock restarts when this changes.
    moved: u64,
    /// The connection broke or stalled: close without ceremony.
    broken: bool,
}

impl Pending {
    /// Whether there is nothing more to carry: the handler is done and all it wrote is passed on.
    fn finished(&self) -> bool {
        self.app_done && self.from_app.is_empty()
    }

    /// Whether the handler's input should end now: the client is done and the handler has all its bytes.
    fn input_ends(&self) -> bool {
        self.client_done && self.to_app.is_empty() && !self.app_eof_sent
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

/// How long one poll waits at most.
const POLL_MS: libc::c_int = 1000;

/// The descriptor to give `poll` for this interest: -1 (skipped) when nobody asked about it.
fn poll_fd(i: Interest) -> libc::c_int {
    if i.read || i.write {
        i.stream.as_raw_fd()
    } else {
        -1
    }
}

/// Wait for readiness of two descriptors, in one call (the one place this module polls). A descriptor with no interest is
/// given as -1, which `poll` skips: a descriptor that has hung up is reported even when no event was asked for, and
/// listing it would return at once, every time.
fn wait(a: Interest, b: Interest) {
    let mut fds = [a, b].map(|i| libc::pollfd {
        fd: poll_fd(i),
        events: if i.read { libc::POLLIN } else { 0 } | if i.write { libc::POLLOUT } else { 0 },
        revents: 0,
    });
    // SAFETY: fds is a valid array of two pollfd for the duration of the call.
    unsafe {
        libc::poll(fds.as_mut_ptr(), 2, POLL_MS);
    }
}

/// The three things a bridge holds: the TLS connection, its socket, and the local end for the handler.
struct Bridge<'a> {
    conn: ServerConnection,
    sock: &'a mut Stream,
    app: Stream,
    p: Pending,
}

/// Carry bytes between the TLS connection and the local end `app` until the handler is done, an error breaks the
/// connection, or nothing has moved for `stall`.
pub fn bridge(conn: ServerConnection, sock: &mut Stream, app: Stream, stall: Duration) {
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
    let (mut seen, mut since) = (0, Instant::now());
    while !b.p.finished() {
        if b.turn(&mut buf).is_err() {
            b.p.broken = true;
            break;
        }
        if b.p.moved != seen {
            (seen, since) = (b.p.moved, Instant::now());
        } else if since.elapsed() >= stall {
            b.p.broken = true;
            break;
        }
    }
    b.finish();
}

impl Bridge<'_> {
    /// One pass: write what is ready, wait, read what arrived.
    fn turn(&mut self, buf: &mut [u8]) -> Result<(), ()> {
        let Bridge { conn, sock, app, p } = self;
        p.moved += flush_tls(conn, sock).map_err(|_| ())? as u64;
        pass_to_tls(conn, p).map_err(|_| ())?;
        pass_to_app(app, p).map_err(|_| ())?;
        if p.input_ends() {
            // everything the client sent is with the handler: the handler's input ends here
            p.app_eof_sent = true;
            let _ = app.shutdown(std::net::Shutdown::Write);
        }
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
        if self.p.app_done && !self.p.broken {
            self.close_notify();
            if !self.p.client_done {
                drain(self.sock);
            }
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
fn flush_tls(conn: &mut ServerConnection, sock: &mut Stream) -> std::io::Result<usize> {
    let mut written = 0;
    while conn.wants_write() {
        match conn.write_tls(sock) {
            Ok(0) => break,
            Ok(n) => written += n,
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(e) if retry(&e) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(written)
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
        Ok(n) => {
            p.to_app.drain(..n);
        }
        Err(e) if retry(&e) => {}
        Err(e) => return Err(e),
    }
    Ok(())
}

/// Take TLS records from the socket: whether the client closed the connection.
fn take_records(
    conn: &mut ServerConnection,
    sock: &mut Stream,
    p: &mut Pending,
) -> Result<bool, ()> {
    match conn.read_tls(sock) {
        Ok(0) => Ok(true),
        Ok(n) => {
            p.moved += n as u64;
            Ok(false)
        }
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
    p.client_done |= take_records(conn, sock, p)?;
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

/// Read and drop what the client is still sending, up to 256 KiB and at most two seconds in all (a client that trickles
/// one byte at a time cannot hold the socket longer: the first version bounded only the bytes and could take about 13
/// seconds). After the answer, `close_notify` is flushed for at most two seconds first, so the close comes at most about
/// four seconds after the handler finished.
fn drain(sock: &mut Stream) {
    let _ = sock.set_nonblocking(false);
    let _ = sock.set_read_timeout(Some(Duration::from_millis(200)));
    let until = Instant::now() + Duration::from_secs(2);
    let mut sink = [0u8; 4096];
    let mut left = 256 * 1024usize;
    while left > 0 && Instant::now() < until {
        if sock.read(&mut sink).is_ok_and(|n| n > 0) {
            left = left.saturating_sub(sink.len());
        } else {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loopback::connected_pair;

    #[test]
    fn a_descriptor_nobody_asked_about_is_left_out_of_the_poll() {
        // a hung-up descriptor is reported even when no events were requested (Linux does; macOS does not always), so a
        // loop that lists it can return at once, every time, and spin: the poll gets -1 for it, which poll skips
        let (stream, _other) = connected_pair().unwrap();
        let ask = |read, write| Interest {
            stream: &stream,
            read,
            write,
        };
        assert_eq!(poll_fd(ask(false, false)), -1);
        for (read, write) in [(true, false), (false, true), (true, true)] {
            assert_eq!(poll_fd(ask(read, write)), stream.as_raw_fd());
        }
        // and nothing listed means the call waits its full time: it does not return at once
        let (quiet, _held) = connected_pair().unwrap();
        let idle = |stream| Interest {
            stream,
            read: false,
            write: false,
        };
        let started = Instant::now();
        wait(idle(&stream), idle(&quiet));
        assert!(started.elapsed() >= Duration::from_millis(800));
    }

    #[test]
    fn only_bytes_on_the_client_socket_count_as_movement_not_what_passes_between_the_pump_and_the_handler()
     {
        // the handler's side of the pump: bytes taken from it and bytes given to it move nothing the stall clock sees,
        // so a client that accepts nothing is not kept alive by the handler's own output (or its heartbeat)
        let (mut app, mut handler) = connected_pair().unwrap();
        app.set_nonblocking(true).unwrap();
        let mut p = Pending::default();
        let mut buf = [0u8; 64];
        handler.write_all(b"heartbeat").unwrap();
        read_app(&mut app, &mut p, &mut buf);
        assert_eq!(p.from_app, b"heartbeat");
        assert_eq!(p.moved, 0, "handler bytes counted as movement");
        p.to_app = b"request".to_vec();
        pass_to_app(&mut app, &mut p).unwrap();
        assert!(p.to_app.is_empty());
        assert_eq!(p.moved, 0, "bytes given to the handler counted as movement");
    }
}
