//! What decides, before a client has shown a certificate, whether its connection is kept (#1493): the key a source address
//! is counted under, the places of established sessions (apart from the handshake pool of `tls_limits.rs`), and the one
//! log line that says connections were refused.
//!
//! **The source key.** An IPv4 address is counted as itself (a /32). An IPv6 address is counted by its first 64 bits (a
//! /64): a host or a site is given a whole /64, so counting each /128 would let one machine hold as many places as it has
//! addresses. An IPv4-mapped IPv6 address (`::ffff:a.b.c.d`) is counted as the IPv4 address it carries, so a dual-stack
//! listener cannot be used to count one host twice. The table that counts keys lives in `tls_limits.rs`.
//!
//! **Sessions.** The number of connections that finished the handshake is capped at `--max-connections`. A connection still
//! in the handshake is not counted here, so a flood of silent sockets cannot take the place of a client that has a
//! certificate; a connection past the cap is closed, as the cap has always closed it.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// The address a connection is counted under (see the module doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKey {
    V4([u8; 4]),
    V6Net([u8; 8]),
}

impl SourceKey {
    /// The key of an address given as 16 octets (an IPv4 address in its IPv4-mapped IPv6 form, as
    /// `loopback::peer_octets` gives it).
    pub fn of(o: [u8; 16]) -> SourceKey {
        if o[..10] == [0; 10] && o[10] == 0xff && o[11] == 0xff {
            SourceKey::V4([o[12], o[13], o[14], o[15]])
        } else {
            SourceKey::V6Net([o[0], o[1], o[2], o[3], o[4], o[5], o[6], o[7]])
        }
    }
}

/// The places of connections that finished the handshake.
pub struct SessionSlots {
    max: usize,
    open: AtomicUsize,
}

/// A place among the sessions; giving it up (the end of the connection, or an unwind) frees it.
pub struct Session<'a>(&'a SessionSlots);

impl Drop for Session<'_> {
    fn drop(&mut self) {
        self.0.open.fetch_sub(1, Ordering::SeqCst);
    }
}

impl SessionSlots {
    pub fn new(max: usize) -> SessionSlots {
        SessionSlots {
            max,
            open: AtomicUsize::new(0),
        }
    }

    /// A place, or `None` when `max` sessions are open.
    pub fn claim(&self) -> Option<Session<'_>> {
        let mut n = self.open.load(Ordering::SeqCst);
        loop {
            if n >= self.max {
                return None;
            }
            match self
                .open
                .compare_exchange(n, n + 1, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return Some(Session(self)),
                Err(now) => n = now,
            }
        }
    }

    #[cfg(test)]
    pub fn open(&self) -> usize {
        self.open.load(Ordering::SeqCst)
    }
}

/// How often the refusals are summarised in the log.
const EVERY: Duration = Duration::from_secs(10);

/// Connections refused before the handshake (the pool full, or a source at its share), counted, and said at most once
/// every ten seconds as one line with a count and no address: a flood must not become a flood of log lines, and an
/// address in a log line is something the operator did not ask to keep.
#[derive(Default)]
pub struct Refusals {
    unsaid: AtomicU64,
    total: AtomicU64,
    last: Mutex<Option<Instant>>,
}

impl Refusals {
    /// Every refusal since the start (the log line summarises, this does not).
    #[cfg(test)]
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::SeqCst)
    }

    pub fn note(&self) {
        self.note_at(Instant::now(), &mut std::io::stderr());
    }

    fn note_at(&self, now: Instant, out: &mut dyn std::io::Write) {
        self.unsaid.fetch_add(1, Ordering::SeqCst);
        self.total.fetch_add(1, Ordering::SeqCst);
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|t| now.saturating_duration_since(t) < EVERY) {
            return;
        }
        *last = Some(now);
        let n = self.unsaid.swap(0, Ordering::SeqCst);
        let _ = writeln!(
            out,
            "klatch-server: https: {n} connection(s) refused before the handshake (handshake pool full or a source address at its share); at most one such line every {} seconds",
            EVERY.as_secs()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An IPv4 address in the form the key is made from.
    fn v4(a: u8, b: u8, c: u8, d: u8) -> [u8; 16] {
        let mut o = [0u8; 16];
        o[10] = 0xff;
        o[11] = 0xff;
        o[12..].copy_from_slice(&[a, b, c, d]);
        o
    }

    /// An IPv6 address from its first four groups and last group (`2001:db8:g2:g3::last`-like).
    fn v6(g: [u16; 4], last: u16) -> [u8; 16] {
        let mut o = [0u8; 16];
        for (i, x) in g.iter().enumerate() {
            o[2 * i..2 * i + 2].copy_from_slice(&x.to_be_bytes());
        }
        o[14..].copy_from_slice(&last.to_be_bytes());
        o
    }

    #[test]
    fn an_ipv4_address_is_its_own_key_and_an_ipv6_address_is_counted_by_its_64() {
        let a = SourceKey::of(v4(192, 0, 2, 1));
        assert_eq!(a, SourceKey::of(v4(192, 0, 2, 1)));
        assert_ne!(a, SourceKey::of(v4(192, 0, 2, 2)));
        let x = SourceKey::of(v6([0x2001, 0xdb8, 1, 2], 1));
        let mut same = v6([0x2001, 0xdb8, 1, 2], 0xffff);
        same[8..14].copy_from_slice(&[0xff; 6]);
        assert_eq!(x, SourceKey::of(same), "one /64 is one source");
        assert_ne!(x, SourceKey::of(v6([0x2001, 0xdb8, 1, 3], 1)));
        assert_ne!(a, x);
    }

    #[test]
    fn an_ipv4_mapped_address_is_the_ipv4_address_and_a_v6_one_that_only_looks_like_it_is_not() {
        assert_eq!(
            SourceKey::of(v4(192, 0, 2, 1)),
            SourceKey::V4([192, 0, 2, 1])
        );
        // the same low 32 bits under another prefix are an IPv6 network
        let mut o = v4(192, 0, 2, 1);
        o[0] = 1;
        assert!(matches!(SourceKey::of(o), SourceKey::V6Net(_)));
    }

    #[test]
    fn sessions_are_capped_and_a_place_is_given_back_when_dropped() {
        let slots = SessionSlots::new(2);
        let a = slots.claim().unwrap();
        let _b = slots.claim().unwrap();
        assert!(slots.claim().is_none());
        drop(a);
        assert_eq!(slots.open(), 1);
        let _c = slots.claim().expect("the place came back");
        assert!(slots.claim().is_none());
        // a panic gives the place back too
        let slots = SessionSlots::new(1);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _s = slots.claim().unwrap();
            panic!("inside a session");
        }));
        assert!(r.is_err());
        assert_eq!(slots.open(), 0);
    }

    #[test]
    fn refusals_are_summarised_once_per_interval_without_an_address() {
        let r = Refusals::default();
        let t0 = Instant::now();
        let mut out = Vec::new();
        for _ in 0..1000 {
            r.note_at(t0, &mut out);
        }
        let text = String::from_utf8(out.clone()).unwrap();
        assert_eq!(text.lines().count(), 1, "a flood made {text:?}");
        assert!(!text.contains("127.") && !text.contains("::"), "{text}");
        r.note_at(t0 + EVERY, &mut out);
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.lines().next().unwrap().contains(": 1 connection"));
        assert!(text.lines().nth(1).unwrap().contains(": 1000 connection"));
    }
}
