//! Forwarded identity headers: ignored, and counted by name (#110, epic #13).
//!
//! Behind a reverse proxy or a TLS terminator a request may carry the proxy's word about the caller: an address, a user,
//! a client certificate. klatch never takes that word (the owner's decision of 2026-11-23): the one identity is the
//! worker credential, so these headers decide nothing. This module only notices that one was sent, so an operator can see
//! a proxy adding them, or a client forging them.
//!
//! The rule that keeps it safe: this module is handed header **names** and nothing else. It never sees a value, so it
//! cannot store or log an address, a user or a certificate. `tests/route_audit.rs` holds that line: this is the only
//! source file allowed to spell these names, and it must not name a header type or a value accessor.
//!
//! The counts live in memory, for the life of one server process. `forwarded_report.rs` shows them (a counts-only line
//! on stderr while the server runs and when it stops) and keeps them (one audit row per interval, from a timer, never
//! from a request) so they survive a restart (#1490).

use std::sync::atomic::{AtomicU64, Ordering};

/// The fixed list, lower case. A request is counted under each of these names it carries (once per request, however
/// often the header repeats). The documented list is in `clients/recipes/behind-a-tunnel.md`, section 1.
pub(crate) const NAMES: &[&str] = &[
    "forwarded",
    "x-forwarded-for",
    "x-forwarded-user",
    "x-forwarded-email",
    "x-forwarded-client-cert",
    "x-real-ip",
    "x-remote-user",
    "x-client-cert",
    "x-client-verify",
    "x-amzn-oidc-identity",
];

/// One more name, a prefix family (`X-SSL-Client-S-DN`, `X-SSL-Client-Verify`, ...): all of them count as this one name.
const SSL_PREFIX: &str = "x-ssl-client-";
/// How the prefix family is named in the summary.
const SSL_LABEL: &str = "x-ssl-client-*";

/// The counters: requests that carried at least one, and per name how many requests carried it.
#[derive(Default)]
pub(crate) struct Ignored {
    requests: AtomicU64,
    by_name: [AtomicU64; NAMES.len() + 1],
    /// How often `snapshot` was called, so a test can tell a timer that sleeps from one that spins.
    #[cfg(test)]
    looks: AtomicU64,
}

/// The slot of a header name, or `None` when it is not one of the list.
fn slot(name: &str) -> Option<usize> {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with(SSL_PREFIX) {
        return Some(NAMES.len());
    }
    NAMES.iter().position(|n| *n == lower)
}

/// One more, stopping at the top instead of wrapping to zero.
fn sat_inc(c: &AtomicU64, store: Ordering) {
    let _ = c.fetch_update(store, Ordering::Relaxed, |n| Some(n.saturating_add(1)));
}

impl Ignored {
    /// Count `n` requests under the first listed name, for tests of what the counts do (not of what they name).
    #[cfg(test)]
    pub(crate) fn bump(&self, n: u64) {
        self.requests.fetch_add(n, Ordering::Relaxed);
        self.by_name[0].fetch_add(n, Ordering::Relaxed);
    }

    /// How many times `snapshot` has been called.
    #[cfg(test)]
    pub(crate) fn looks(&self) -> u64 {
        self.looks.load(Ordering::Relaxed)
    }

    /// Count one request given the names of its headers. Only names; a value is never passed in.
    pub(crate) fn note<'a>(&self, names: impl IntoIterator<Item = &'a str>) {
        let mut seen = [false; NAMES.len() + 1];
        for i in names.into_iter().filter_map(slot) {
            seen[i] = true;
        }
        if !seen.contains(&true) {
            return;
        }
        // requests first, then each name with Release: a reader that sees the name count (Acquire, in `snapshot`) also
        // sees the request that raised it
        sat_inc(&self.requests, Ordering::Relaxed);
        for (i, _) in seen.iter().enumerate().filter(|(_, hit)| **hit) {
            sat_inc(&self.by_name[i], Ordering::Release);
        }
    }

    /// The counts as they stand: plain numbers, safe to keep, compare and render. The names are read before the
    /// requests, so no name count is above `requests` (a request that is mid-count may show in `requests` and not yet
    /// in its names).
    pub(crate) fn snapshot(&self) -> Counts {
        #[cfg(test)]
        self.looks.fetch_add(1, Ordering::Relaxed);
        let by_name = std::array::from_fn(|i| self.by_name[i].load(Ordering::Acquire));
        Counts {
            requests: self.requests.load(Ordering::Acquire),
            by_name,
        }
    }

    /// The counts-only line at shutdown: requests, then each name that was seen with its count. No value, no address.
    pub(crate) fn summary(&self) -> String {
        format!(
            "klatch-server: forwarded identity headers ignored: {}",
            self.snapshot().render()
        )
    }
}

/// A reading of the counters: numbers only, no name text but the fixed list's.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub(crate) struct Counts {
    requests: u64,
    by_name: [u64; NAMES.len() + 1],
}

impl Counts {
    /// Nothing counted.
    pub(crate) fn is_zero(&self) -> bool {
        self.requests == 0
    }

    /// What was counted since `earlier` (an earlier reading of the same counters): each number minus its old value.
    pub(crate) fn since(&self, earlier: &Counts) -> Counts {
        Counts {
            requests: self.requests.saturating_sub(earlier.requests),
            by_name: std::array::from_fn(|i| self.by_name[i].saturating_sub(earlier.by_name[i])),
        }
    }

    /// `requests=N name=K ...`: each name that was seen, in the fixed order. Lower case letters, digits, `=`, `-`,
    /// `*` and spaces only, so it is safe to log and to keep as a row.
    pub(crate) fn render(&self) -> String {
        let mut line = format!("requests={}", self.requests);
        for (i, n) in self.by_name.iter().enumerate().filter(|(_, n)| **n > 0) {
            let name = NAMES.get(i).copied().unwrap_or(SSL_LABEL);
            line.push_str(&format!(" {name}={n}"));
        }
        line
    }

    /// A reading with `n` requests, all under the first listed name (for tests of what counts do, not of what they name).
    #[cfg(test)]
    pub(crate) fn of(n: u64) -> Counts {
        let mut c = Counts {
            requests: n,
            ..Counts::default()
        };
        c.by_name[0] = n;
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_without_one_changes_nothing() {
        let c = Ignored::default();
        c.note([
            "host",
            "authorization",
            "x-klatch-member",
            "x-forwarded-proto",
        ]);
        assert_eq!(
            c.summary(),
            "klatch-server: forwarded identity headers ignored: requests=0"
        );
    }

    #[test]
    fn a_request_counts_once_and_each_name_once_however_often_it_repeats() {
        let c = Ignored::default();
        c.note(["X-Forwarded-For", "x-forwarded-for", "Forwarded", "host"]);
        c.note(["x-forwarded-for"]);
        let s = c.summary();
        assert!(s.contains("requests=2"), "{s}");
        assert!(s.contains("x-forwarded-for=2"), "{s}");
        assert!(s.contains("forwarded=1"), "{s}");
    }

    #[test]
    fn the_certificate_prefix_family_is_one_name() {
        let c = Ignored::default();
        c.note([
            "X-SSL-Client-S-DN",
            "x-ssl-client-verify",
            "x-ssl-client-cert",
        ]);
        assert!(c.summary().ends_with("requests=1 x-ssl-client-*=1"));
    }

    #[test]
    fn every_listed_name_is_found_and_has_its_own_slot() {
        for (i, name) in NAMES.iter().enumerate() {
            assert_eq!(slot(name), Some(i), "{name}");
            assert_eq!(slot(&name.to_ascii_uppercase()), Some(i), "{name}");
        }
        assert_eq!(slot("x-ssl-client-anything"), Some(NAMES.len()));
        assert_eq!(slot("x-klatch-member"), None);
    }

    #[test]
    fn a_counter_at_the_top_stays_at_the_top() {
        let c = AtomicU64::new(u64::MAX - 1);
        sat_inc(&c, Ordering::Relaxed);
        sat_inc(&c, Ordering::Relaxed);
        sat_inc(&c, Ordering::Relaxed);
        assert_eq!(c.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn a_snapshot_never_shows_a_name_above_the_requests_even_while_requests_arrive() {
        let ignored = std::sync::Arc::new(Ignored::default());
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writers: Vec<_> = (0..3)
            .map(|_| {
                let (i, s) = (ignored.clone(), stop.clone());
                std::thread::spawn(move || {
                    while !s.load(Ordering::Relaxed) {
                        i.note(["x-real-ip", "forwarded"]);
                    }
                })
            })
            .collect();
        for _ in 0..200_000 {
            let c = ignored.snapshot();
            assert!(
                c.by_name.iter().all(|n| *n <= c.requests),
                "a name count above requests: {c:?}"
            );
        }
        stop.store(true, Ordering::Relaxed);
        for w in writers {
            w.join().unwrap();
        }
    }
}
