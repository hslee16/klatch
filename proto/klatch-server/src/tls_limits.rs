//! The limits of the mutual TLS listener that bound what a peer with no certificate can do (#108): a handshake deadline,
//! a cap on connections that are still in the handshake, and the stall time of the pump (`tls_bridge.rs`).
//!
//! A peer that has no certificate can still open a TCP connection, and until its handshake fails or times out it holds a
//! connection slot. Two bounds keep that from starving everyone else. The handshake has a short deadline. And at most
//! `max_handshakes` connections are in the handshake at once; when one more arrives, the OLDEST handshake in progress is
//! closed to make room (a real client finishes its handshake in milliseconds, so the oldest is the silent or slow one).
//! A flood of silent sockets therefore cannot keep a valid client out: the newcomer always gets a place. What stays is a
//! denial of service by volume (a flood fast enough to evict a valid handshake while it is in flight, or one that fills
//! the connection cap with connections that did complete the handshake, which needs a certificate): any peer that can
//! reach the port can try, which is why the listener is loopback unless `--allow-off-loopback` is given, and a firewall
//! or a tunnel in front of it is advised.

use crate::loopback::Stream;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

/// The default time a client has to finish the handshake.
pub const DEFAULT_HANDSHAKE: Duration = Duration::from_secs(3);
/// The default time a connection may move no byte before it is closed. More than the event stream's heartbeat (15 s).
pub const DEFAULT_STALL: Duration = Duration::from_secs(60);

/// What the operator can tune.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Connections that may be in the handshake at once.
    pub max_handshakes: usize,
    pub handshake: Duration,
    pub stall: Duration,
}

impl Limits {
    /// A quarter of the connection cap (at least one) may be in the handshake.
    pub fn defaults(max_connections: usize) -> Limits {
        Limits {
            max_handshakes: (max_connections / 4).max(1),
            handshake: DEFAULT_HANDSHAKE,
            stall: DEFAULT_STALL,
        }
    }
}

/// The connections in the handshake, oldest first.
#[derive(Default)]
pub struct Handshakes {
    open: Mutex<(u64, VecDeque<(u64, Stream)>)>,
}

impl Handshakes {
    /// A connection begins its handshake. When `max` are already in progress, the oldest is shut (its thread's read or
    /// write fails at once). The returned number is given back to [`Handshakes::leave`]; `None` when the connection
    /// cannot be tracked (then it is not served).
    pub fn enter(&self, sock: &Stream, max: usize) -> Option<u64> {
        let handle = sock.try_clone().ok()?;
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        open.0 += 1;
        let id = open.0;
        open.1.push_back((id, handle));
        while open.1.len() > max {
            if let Some((_, oldest)) = open.1.pop_front() {
                let _ = oldest.shutdown(std::net::Shutdown::Both);
            }
        }
        Some(id)
    }

    /// The handshake of connection `id` is over, one way or the other.
    pub fn leave(&self, id: u64) {
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        open.1.retain(|(i, _)| *i != id);
    }

    /// How many handshakes are in progress.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.open.lock().unwrap_or_else(|e| e.into_inner()).1.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loopback::connected_pair;

    #[test]
    fn a_handshake_over_the_cap_closes_the_oldest_and_never_the_newcomer() {
        let guard = Handshakes::default();
        let pairs: Vec<_> = (0..3).map(|_| connected_pair().unwrap()).collect();
        let ids: Vec<u64> = pairs
            .iter()
            .map(|(a, _)| guard.enter(a, 2).unwrap())
            .collect();
        assert_eq!(guard.len(), 2);
        // the first was shut: its peer end reads the end of the stream; the last two are open
        use std::io::Read;
        let mut buf = [0u8; 1];
        let mut first_peer = pairs[0].1.try_clone().unwrap();
        assert_eq!(first_peer.read(&mut buf).unwrap(), 0);
        pairs[2].1.set_nonblocking(true).unwrap();
        let mut last_peer = pairs[2].1.try_clone().unwrap();
        assert_eq!(
            last_peer.read(&mut buf).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        guard.leave(ids[1]);
        guard.leave(ids[2]);
        assert_eq!(guard.len(), 0);
    }

    #[test]
    fn the_default_cap_is_a_quarter_of_the_connections_and_at_least_one() {
        assert_eq!(Limits::defaults(32).max_handshakes, 8);
        assert_eq!(Limits::defaults(3).max_handshakes, 1);
        assert_eq!(Limits::defaults(1).max_handshakes, 1);
        assert!(Limits::defaults(32).handshake < std::time::Duration::from_secs(5));
    }
}
