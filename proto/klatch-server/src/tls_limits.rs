//! The limits of the mutual TLS listener that bound what a peer with no certificate can do (#108): a handshake deadline,
//! a cap on connections that are still in the handshake, and the stall time of the pump (`tls_bridge.rs`).
//!
//! A peer that has no certificate can still open a TCP connection, and until its handshake fails or times out it holds a
//! connection slot. The handshake has a short deadline, and at most `max_handshakes` connections are in the handshake at
//! once. What happens to a newcomer when the cap is full depends on age, and one side always loses:
//!
//! - every handshake in progress is YOUNGER than the minimum age (half the deadline): the NEWCOMER is refused (closed at
//!   once). An honest client that is mid-handshake is never closed to make room, so a flood of silent sockets cannot
//!   evict it; the price is that a newcomer arriving while the cap is full of young handshakes (a flood has just
//!   arrived) is turned away, honest or not, until some handshake is old enough or ends;
//! - the OLDEST handshake is at least the minimum age: it is closed and the newcomer takes its place. A real handshake
//!   takes milliseconds, so one that old is silent or crawling; a flood of silent sockets is cleared this way, one
//!   newcomer at a time, and a valid client gets in once the flood is older than the minimum age.
//!
//! This bounds the flood; it does not remove denial of service by volume. A peer that can reach the port and keeps the
//! cap full of fresh connections (opening one whenever one ends) keeps newcomers out, and an honest client that is merely
//! slow (older than the minimum age with the deadline not yet reached) can be evicted by a newcomer.
//!
//! **The real cost of that is low.** A peer with NO certificate needs only a few connections per second to keep the cap
//! full: a reviewer starved 10 of 10 honest clients at a cap of 4 with about 3.3 connections per second, and with the
//! defaults (a cap of 8, a 3 s deadline, a 1.5 s minimum age) it is roughly 5 to 6 per second. It was equally cheap
//! before the age rule; what the rule changed is that a client already mid-handshake is no longer evicted. The remedies
//! are outside this module: the listener is loopback unless `--allow-off-loopback` is given, a firewall or a tunnel in
//! front of it is advised, and a per-source limit is #1493.

use crate::loopback::Stream;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
    /// A handshake younger than this is never closed to make room for another connection.
    pub fn min_evict_age(&self) -> Duration {
        self.handshake / 2
    }

    /// A quarter of the connection cap (at least one) may be in the handshake.
    pub fn defaults(max_connections: usize) -> Limits {
        Limits {
            max_handshakes: (max_connections / 4).max(1),
            handshake: DEFAULT_HANDSHAKE,
            stall: DEFAULT_STALL,
        }
    }
}

/// The connections in the handshake, oldest first: their number, when each began, and a handle to shut it.
#[derive(Default)]
struct Open {
    next: u64,
    queue: VecDeque<(u64, Instant, Stream)>,
}

#[derive(Default)]
pub struct Handshakes {
    open: Mutex<Open>,
}

/// A place among the handshakes in progress. Giving it up (the end of the handshake, or an unwind out of it) frees the
/// place and the handle, so a panic cannot leak either.
pub struct Ticket<'a> {
    guard: &'a Handshakes,
    id: u64,
}

impl Drop for Ticket<'_> {
    fn drop(&mut self) {
        self.guard.leave(self.id);
    }
}

impl Handshakes {
    /// A connection begins its handshake. With `max` already in progress: if the oldest is at least `min_age` old it is
    /// shut (its thread's read or write fails at once) and the newcomer takes a place; if every one is younger, the
    /// newcomer is refused (`None`). `None` is also what a connection that cannot be tracked gets.
    pub fn enter(&self, sock: &Stream, max: usize, min_age: Duration) -> Option<Ticket<'_>> {
        let handle = sock.try_clone().ok()?;
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        if open.queue.len() >= max {
            let old_enough = open
                .queue
                .front()
                .is_some_and(|(_, at, _)| at.elapsed() >= min_age);
            if !old_enough {
                return None;
            }
            if let Some((_, _, oldest)) = open.queue.pop_front() {
                let _ = oldest.shutdown(std::net::Shutdown::Both);
            }
        }
        open.next += 1;
        let id = open.next;
        open.queue.push_back((id, Instant::now(), handle));
        Some(Ticket { guard: self, id })
    }

    fn leave(&self, id: u64) {
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        open.queue.retain(|(i, _, _)| *i != id);
    }

    /// How many handshakes are in progress.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.open
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .queue
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loopback::connected_pair;
    use std::io::Read;

    const NEVER: Duration = Duration::from_secs(3600);

    #[test]
    fn a_newcomer_over_the_cap_is_refused_while_every_handshake_is_younger_than_the_minimum_age() {
        let guard = Handshakes::default();
        let (first, first_peer) = connected_pair().unwrap();
        let (second, _second_peer) = connected_pair().unwrap();
        let held = guard.enter(&first, 1, NEVER).expect("the first fits");
        assert!(
            guard.enter(&second, 1, NEVER).is_none(),
            "the newcomer is refused"
        );
        assert_eq!(guard.len(), 1);
        // the first is untouched: its peer has not seen the end of the stream
        first_peer.set_nonblocking(true).unwrap();
        let mut buf = [0u8; 1];
        let mut peer = first_peer.try_clone().unwrap();
        assert_eq!(
            peer.read(&mut buf).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        drop(held);
        assert_eq!(guard.len(), 0);
    }

    #[test]
    fn a_handshake_older_than_the_minimum_age_is_closed_to_admit_a_newcomer() {
        let guard = Handshakes::default();
        let (first, mut first_peer) = connected_pair().unwrap();
        let (second, _second_peer) = connected_pair().unwrap();
        let _old = guard.enter(&first, 1, Duration::ZERO).unwrap();
        let _new = guard
            .enter(&second, 1, Duration::ZERO)
            .expect("the old one makes room");
        assert_eq!(guard.len(), 1);
        let mut buf = [0u8; 1];
        assert_eq!(
            first_peer.read(&mut buf).unwrap(),
            0,
            "the evicted one was shut"
        );
    }

    #[test]
    fn a_panic_during_a_handshake_still_gives_back_the_slot_and_the_descriptor() {
        let guard = Handshakes::default();
        let (sock, mut peer) = connected_pair().unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ticket = guard.enter(&sock, 4, NEVER).unwrap();
            assert_eq!(guard.len(), 1);
            panic!("inside the handshake");
        }));
        assert!(result.is_err());
        assert_eq!(guard.len(), 0, "the slot leaked");
        // the guard's own copy of the descriptor is closed: with the socket dropped too, the peer sees the end
        drop(sock);
        let mut buf = [0u8; 1];
        assert_eq!(peer.read(&mut buf).unwrap(), 0, "a descriptor leaked");
    }

    #[test]
    fn the_default_cap_is_a_quarter_of_the_connections_and_the_minimum_age_half_the_deadline() {
        assert_eq!(Limits::defaults(32).max_handshakes, 8);
        assert_eq!(Limits::defaults(3).max_handshakes, 1);
        assert_eq!(Limits::defaults(1).max_handshakes, 1);
        let d = Limits::defaults(32);
        assert!(d.handshake < Duration::from_secs(5));
        assert_eq!(d.min_evict_age(), d.handshake / 2);
    }
}
