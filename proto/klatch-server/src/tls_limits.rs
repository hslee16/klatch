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
//! front of it is advised, and one source address may hold only `per_source` of the places (#1493, below and in
//! `tls_admit.rs`, which also keeps the session cap apart from this pool).

use crate::loopback::Stream;
use crate::tls_admit::{Refusals, SourceKey};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The default time a client has to finish the handshake.
pub const DEFAULT_HANDSHAKE: Duration = Duration::from_secs(3);
/// The default time a connection may move no byte before it is closed. More than the event stream's heartbeat (15 s).
pub const DEFAULT_STALL: Duration = Duration::from_secs(60);

/// The default share of the handshake pool for one source address. A handshake lasts milliseconds, so two at once is
/// ample for an honest host; more than the pool (when the pool is smaller) is clamped by [`Limits::per_source_within`].
pub const DEFAULT_PER_SOURCE: usize = 2;

/// What the operator can tune.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Connections that may be in the handshake at once.
    pub max_handshakes: usize,
    /// Handshakes that one source address (an IPv4 /32, an IPv6 /64) may have in progress at once.
    pub per_source: usize,
    pub handshake: Duration,
    pub stall: Duration,
}

impl Limits {
    /// A handshake younger than this is never closed to make room for another connection.
    pub fn min_evict_age(&self) -> Duration {
        self.handshake / 2
    }

    /// `per_source` never exceeds the pool: a share larger than the pool means nothing.
    pub fn per_source_within(&self) -> usize {
        self.per_source.min(self.max_handshakes)
    }

    /// A quarter of the connection cap (at least one) may be in the handshake.
    pub fn defaults(max_connections: usize) -> Limits {
        Limits {
            max_handshakes: (max_connections / 4).max(1),
            per_source: DEFAULT_PER_SOURCE,
            handshake: DEFAULT_HANDSHAKE,
            stall: DEFAULT_STALL,
        }
    }
}

/// The connections in the handshake, oldest first: their number, when each began, and a handle to shut it.
#[derive(Default)]
struct Open {
    next: u64,
    queue: VecDeque<(u64, Instant, SourceKey, Stream)>,
    /// How many handshakes each source has in progress. An entry exists only while its count is above zero, so the table
    /// holds at most one entry per handshake in progress: never more than the pool (`max_handshakes`), whatever number of
    /// distinct sources there are. There is nothing to expire.
    sources: HashMap<SourceKey, usize>,
}

impl Open {
    fn forget(&mut self, key: SourceKey) {
        if let Some(n) = self.sources.get_mut(&key) {
            *n -= 1;
            if *n == 0 {
                self.sources.remove(&key);
            }
        }
    }
}

#[derive(Default)]
pub struct Handshakes {
    open: Mutex<Open>,
    pub refusals: Refusals,
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
    /// A connection begins its handshake. It is refused (`None`) when its source already has `per_source` handshakes in
    /// progress and none of them is old enough to close (`min_age`, as below; one address never closes another's).
    /// Otherwise, with `max` already in
    /// progress: if the oldest is at least `min_age` old it is shut (its thread's read or write fails at once) and the
    /// newcomer takes a place; if every one is younger, the newcomer is refused. `None` is also what a connection that
    /// cannot be tracked (no peer address, no handle) gets. A refusal is counted for the log (`Refusals`).
    pub fn enter(
        &self,
        sock: &Stream,
        max: usize,
        per_source: usize,
        min_age: Duration,
    ) -> Option<Ticket<'_>> {
        let key = crate::loopback::peer_octets(sock).map(SourceKey::of);
        let ticket = key.and_then(|k| self.enter_from(sock, k, max, per_source, min_age));
        if ticket.is_none() {
            self.refusals.note();
        }
        ticket
    }

    /// [`Handshakes::enter`] for a source that is named (so a test can stand in for many addresses).
    pub fn enter_from(
        &self,
        sock: &Stream,
        key: SourceKey,
        max: usize,
        per_source: usize,
        min_age: Duration,
    ) -> Option<Ticket<'_>> {
        let handle = sock.try_clone().ok()?;
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        if open.sources.get(&key).copied().unwrap_or(0) >= per_source {
            // the source is at its share: its own oldest handshake gives way if it is old enough (as in the pool, below), so
            // a client behind the same address can get in once the silent ones are old (not against an attacker who keeps
            // opening them: see SECURITY.md); otherwise the newcomer loses
            let at = open
                .queue
                .iter()
                .position(|(_, at, k, _)| *k == key && at.elapsed() >= min_age)?;
            if let Some((_, _, evicted_key, oldest)) = open.queue.remove(at) {
                let _ = oldest.shutdown(std::net::Shutdown::Both);
                open.forget(evicted_key);
            }
        }
        if open.queue.len() >= max {
            let old_enough = open
                .queue
                .front()
                .is_some_and(|(_, at, _, _)| at.elapsed() >= min_age);
            if !old_enough {
                return None;
            }
            if let Some((_, _, evicted_key, oldest)) = open.queue.pop_front() {
                let _ = oldest.shutdown(std::net::Shutdown::Both);
                open.forget(evicted_key);
            }
        }
        open.next += 1;
        let id = open.next;
        open.queue.push_back((id, Instant::now(), key, handle));
        *open.sources.entry(key).or_insert(0) += 1;
        Some(Ticket { guard: self, id })
    }

    fn leave(&self, id: u64) {
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        // an entry that eviction already took out was counted out then
        if let Some(at) = open.queue.iter().position(|(i, ..)| *i == id)
            && let Some((_, _, key, _)) = open.queue.remove(at)
        {
            open.forget(key);
        }
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

    /// How many sources the table holds.
    #[cfg(test)]
    pub fn sources(&self) -> usize {
        self.open
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .sources
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loopback::connected_pair;
    use std::io::Read;

    const NEVER: Duration = Duration::from_secs(3600);

    fn key(n: u8) -> SourceKey {
        let mut o = [0u8; 16];
        o[10] = 0xff;
        o[11] = 0xff;
        o[12..].copy_from_slice(&[192, 0, 2, n]);
        SourceKey::of(o)
    }

    #[test]
    fn a_newcomer_over_the_cap_is_refused_while_every_handshake_is_younger_than_the_minimum_age() {
        let guard = Handshakes::default();
        let (first, first_peer) = connected_pair().unwrap();
        let (second, _second_peer) = connected_pair().unwrap();
        let held = guard
            .enter_from(&first, key(1), 1, 99, NEVER)
            .expect("the first fits");
        assert!(
            guard.enter_from(&second, key(2), 1, 99, NEVER).is_none(),
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
        let _old = guard
            .enter_from(&first, key(1), 1, 99, Duration::ZERO)
            .unwrap();
        let _new = guard
            .enter_from(&second, key(2), 1, 99, Duration::ZERO)
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
            let _ticket = guard.enter_from(&sock, key(1), 4, 99, NEVER).unwrap();
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
        assert_eq!(d.per_source, DEFAULT_PER_SOURCE);
        assert_eq!(Limits::defaults(1).per_source_within(), 1);
    }

    #[test]
    fn one_source_at_its_share_is_refused_and_another_source_still_gets_a_place() {
        let guard = Handshakes::default();
        let socks: Vec<_> = (0..4).map(|_| connected_pair().unwrap()).collect();
        let a1 = guard.enter_from(&socks[0].0, key(1), 8, 2, NEVER).unwrap();
        let _a2 = guard.enter_from(&socks[1].0, key(1), 8, 2, NEVER).unwrap();
        assert!(
            guard.enter_from(&socks[2].0, key(1), 8, 2, NEVER).is_none(),
            "a third place for one source"
        );
        let _b = guard
            .enter_from(&socks[3].0, key(2), 8, 2, NEVER)
            .expect("another source is not held back by the first");
        assert_eq!(guard.len(), 3);
        // the refused one took nothing: a place freed by the source is its own to take again
        drop(a1);
        let again = connected_pair().unwrap();
        assert!(guard.enter_from(&again.0, key(1), 8, 2, NEVER).is_some());
    }

    #[test]
    fn a_source_at_its_share_closes_its_own_old_handshake_but_never_another_sources() {
        let guard = Handshakes::default();
        let socks: Vec<_> = (0..3).map(|_| connected_pair().unwrap()).collect();
        let _other = guard.enter_from(&socks[2].0, key(2), 8, 1, NEVER).unwrap();
        let _mine = guard
            .enter_from(&socks[0].0, key(1), 8, 1, Duration::ZERO)
            .unwrap();
        // old enough: its own oldest makes room
        let (mine_again, _p) = connected_pair().unwrap();
        let new = guard
            .enter_from(&mine_again, key(1), 8, 1, Duration::ZERO)
            .expect("its own old handshake gives way");
        assert_eq!(guard.len(), 2);
        // the evicted handshake was counted out of its source: one of source 1 and one of source 2 are in progress
        assert_eq!(guard.sources(), 2);
        drop(new);
        assert_eq!(guard.sources(), 1, "the self-eviction left a count behind");
        let _again = guard
            .enter_from(&mine_again, key(1), 8, 1, NEVER)
            .expect("the share is free again: nothing of the evicted handshake counts");
        // too young: the newcomer is refused, and source 2's handshake is untouched
        assert!(guard.enter_from(&socks[1].0, key(1), 8, 1, NEVER).is_none());
        assert_eq!(guard.len(), 2);
    }

    #[test]
    fn a_source_refused_by_a_full_pool_is_not_counted() {
        let guard = Handshakes::default();
        let (s1, _p1) = connected_pair().unwrap();
        let (s2, _p2) = connected_pair().unwrap();
        let _held = guard.enter_from(&s1, key(1), 1, 4, NEVER).unwrap();
        assert!(guard.enter_from(&s2, key(2), 1, 4, NEVER).is_none());
        assert_eq!(guard.sources(), 1, "a refused newcomer left an entry");
    }

    #[test]
    fn the_table_never_holds_more_sources_than_the_pool_however_many_distinct_sources_come() {
        let guard = Handshakes::default();
        let pool = 8;
        let (sock, _peer) = connected_pair().unwrap();
        let mut held = Vec::new();
        for n in 0..=255u8 {
            for m in 0..4u8 {
                let k = {
                    let mut o = [0u8; 16];
                    o[10] = 0xff;
                    o[11] = 0xff;
                    o[12..].copy_from_slice(&[10, 0, m, n]);
                    SourceKey::of(o)
                };
                if let Some(t) = guard.enter_from(&sock, k, pool, 2, NEVER) {
                    held.push(t);
                }
                assert!(guard.sources() <= pool, "the table grew past the pool");
                assert!(guard.len() <= pool);
            }
        }
        assert_eq!(guard.len(), pool);
        held.clear();
        assert_eq!((guard.len(), guard.sources()), (0, 0), "the table leaked");
    }

    #[test]
    fn eviction_counts_the_evicted_source_out_once_and_its_ticket_does_not_count_it_out_again() {
        let guard = Handshakes::default();
        let (s1, _p1) = connected_pair().unwrap();
        let (s2, _p2) = connected_pair().unwrap();
        let old = guard.enter_from(&s1, key(1), 1, 2, Duration::ZERO).unwrap();
        let _new = guard
            .enter_from(&s2, key(1), 1, 2, Duration::ZERO)
            .expect("the old one makes room, and its own count with it");
        assert_eq!(guard.sources(), 1);
        drop(old);
        assert_eq!(guard.len(), 1);
        // one handshake of source 1 is in progress: a share of 1 is full
        let (s3, _p3) = connected_pair().unwrap();
        assert!(guard.enter_from(&s3, key(1), 8, 1, NEVER).is_none());
    }

    #[test]
    fn a_socket_with_no_source_cannot_be_counted_and_is_refused() {
        let guard = Handshakes::default();
        let (pair, _peer) = connected_pair().unwrap();
        assert!(guard.enter(&pair, 8, 1, NEVER).is_none());
        assert_eq!(guard.sources(), 0);
        // the refusal went through the counted path (the log line is built from this count)
        assert_eq!(guard.refusals.total(), 1, "a refusal was not counted");
        // a real TCP peer is counted under its address: `tests/tls_flood.rs` (one source holds only its share)
    }
}
