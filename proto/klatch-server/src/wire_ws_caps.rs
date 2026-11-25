//! What the WebSocket route is configured with, and what one member may hold of it (#112, `wire_ws.rs`; the same guard and
//! bounds serve `GET /v1/stream` through `--stream-max-per-member` and `--stream-max-age-secs`, #1517): the origins let
//! through, the deadlines, the cap of connections per member and the maximum age of a connection, with the flags that set
//! them (`--ws-allow-origin`, `--ws-max-per-member`, `--ws-max-age-secs`, on both listeners) and the registry that counts a
//! member's open connections.
//!
//! One valid credential must not hold every slot of `--max-connections`: a connection is counted under the member the
//! credential proved, at the upgrade, and the guard [`Slot`] gives it back whenever the connection function is left, a panic
//! included. The refusal at the cap is the very answer a full server gives anyone (`503 busy`), so it does not say who
//! holds the slots. A connection also ends at a maximum age (default an hour), however lively: a client reconnects and
//! resumes with `subscribe` and `from_seq`.

use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

/// Connections per member, by default (always at most `--max-connections`).
pub const DEFAULT_PER_MEMBER: usize = 4;
/// The longest a connection lives, by default.
pub const DEFAULT_MAX_AGE_SECS: u64 = 3600;
/// The bounds of `--ws-max-age-secs`: a minute (lower only for a debug build with the test knob) to a day.
pub const MAX_AGE_CEILING_SECS: u64 = 86_400;
const MAX_AGE_FLOOR_SECS: u64 = 60;

/// Whether this process may take limits below the shipped minimums: a debug build (what `cargo test` builds) with
/// `KLATCH_TEST_SHORT_LIMITS` set. A release binary has no such knob: the check is compiled out.
pub fn short_limits() -> bool {
    cfg!(debug_assertions) && std::env::var_os("KLATCH_TEST_SHORT_LIMITS").is_some()
}

/// What the WebSocket route is configured with.
#[derive(Debug, Clone)]
pub(crate) struct Settings {
    /// Origins let through, exact (`--ws-allow-origin`); none by default.
    pub(crate) origins: Vec<String>,
    pub(crate) idle: Duration,
    pub(crate) ping: Duration,
    pub(crate) close_wait: Duration,
    /// A shorter stall time than the session's own, for tests only (see the default); `None` is the session's.
    pub(crate) stall: Option<Duration>,
    /// Open connections one member may hold.
    pub(crate) per_member: usize,
    /// How long a connection may live.
    pub(crate) max_age: Duration,
    /// Open `GET /v1/stream` connections one member may hold (#1517): the same guard, counted apart.
    pub(crate) stream_per_member: usize,
    /// How long a stream may live.
    pub(crate) stream_max_age: Duration,
}

impl Default for Settings {
    /// 60 seconds idle, a ping every 20, 2 for a close, the session's 30 seconds for a stalled write, 4 connections per
    /// member, an hour of age; a debug build with `KLATCH_TEST_SHORT_LIMITS` set (as the tests' servers are) takes 3, 1, 1
    /// and 3 seconds for the first four so a test need not wait a minute. A release binary has no such knob.
    fn default() -> Self {
        let short = short_limits();
        let (idle, ping, close) = if short { (3, 1, 1) } else { (60, 20, 2) };
        Settings {
            origins: Vec::new(),
            idle: Duration::from_secs(idle),
            ping: Duration::from_secs(ping),
            close_wait: Duration::from_secs(close),
            stall: short.then_some(Duration::from_secs(3)),
            per_member: DEFAULT_PER_MEMBER,
            max_age: Duration::from_secs(DEFAULT_MAX_AGE_SECS),
            stream_per_member: DEFAULT_PER_MEMBER,
            stream_max_age: Duration::from_secs(DEFAULT_MAX_AGE_SECS),
        }
    }
}

/// The WebSocket flags as given, checked together once every option has been read (the bound of the cap per member is
/// `--max-connections`, which may come later).
#[derive(Default)]
pub struct Raw<'a> {
    origins: Vec<String>,
    per_member: Option<&'a str>,
    max_age: Option<&'a str>,
    stream_per_member: Option<&'a str>,
    stream_max_age: Option<&'a str>,
}

/// The WebSocket options, checked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// Origins let through (`--ws-allow-origin`).
    pub origins: Vec<String>,
    per_member: Option<usize>,
    max_age_secs: Option<u64>,
    stream_per_member: Option<usize>,
    stream_max_age_secs: Option<u64>,
}

/// A cap per member, from 1 to `max_connections`; a bad value names the flag and the bound.
fn parse_cap(flag: &str, text: &str, max_connections: usize) -> Result<usize> {
    text.parse::<usize>()
        .ok()
        .filter(|n| (1..=max_connections).contains(n))
        .ok_or_else(|| {
            anyhow!("{flag} is a whole number from 1 to --max-connections ({max_connections}), not {text:?}")
        })
}

/// A maximum age in seconds, from `floor` to a day; a bad value names the flag and the bounds.
fn parse_age(flag: &str, text: &str, floor: u64) -> Result<u64> {
    text.parse::<u64>()
        .ok()
        .filter(|n| (floor..=MAX_AGE_CEILING_SECS).contains(n))
        .ok_or_else(|| {
            anyhow!("{flag} is a whole number of seconds from {floor} to {MAX_AGE_CEILING_SECS}, not {text:?}")
        })
}

impl<'a> Raw<'a> {
    /// Take one option and its value; an origin is checked at once, a number once the rest is known (`finish`).
    pub fn take(&mut self, flag: &str, value: &'a str) -> Result<()> {
        match flag {
            "--ws-allow-origin" => self.origins.push(parse_origin(value)?),
            "--ws-max-per-member" => self.per_member = Some(value),
            "--stream-max-per-member" => self.stream_per_member = Some(value),
            "--stream-max-age-secs" => self.stream_max_age = Some(value),
            _ => self.max_age = Some(value),
        }
        Ok(())
    }

    /// The options, each number within its bound: a cap per member from 1 to `max_connections`, an age from a minute to a
    /// day (`short` lowers the minimum to a second, for tests). A bad value refuses to start, naming the flag and the bound.
    pub fn finish(self, max_connections: usize, short: bool) -> Result<Options> {
        let cap = |flag: &str, text: Option<&str>| {
            text.map(|text| parse_cap(flag, text, max_connections))
                .transpose()
        };
        let floor = if short && cfg!(debug_assertions) {
            1
        } else {
            MAX_AGE_FLOOR_SECS
        };
        let age = |flag: &str, text: Option<&str>| {
            text.map(|text| parse_age(flag, text, floor)).transpose()
        };
        let per_member = cap("--ws-max-per-member", self.per_member)?;
        let max_age_secs = age("--ws-max-age-secs", self.max_age)?;
        let stream_per_member = cap("--stream-max-per-member", self.stream_per_member)?;
        let stream_max_age_secs = age("--stream-max-age-secs", self.stream_max_age)?;
        Ok(Options {
            origins: self.origins,
            per_member,
            max_age_secs,
            stream_per_member,
            stream_max_age_secs,
        })
    }
}

impl Settings {
    /// Take the options a server was started with.
    pub(crate) fn apply(&mut self, options: &Options, max_connections: usize) {
        self.origins = options.origins.clone();
        self.per_member = options
            .per_member
            .unwrap_or(DEFAULT_PER_MEMBER.min(max_connections));
        if let Some(secs) = options.max_age_secs {
            self.max_age = Duration::from_secs(secs);
        }
        self.stream_per_member = options
            .stream_per_member
            .unwrap_or(DEFAULT_PER_MEMBER.min(max_connections));
        if let Some(secs) = options.stream_max_age_secs {
            self.stream_max_age = Duration::from_secs(secs);
        }
    }
}

/// The host and the port of an origin's authority: a bracketed IPv6 address or a name, then an optional `:port`; and
/// whether the host was bracketed.
fn authority(rest: &str) -> Option<(&str, Option<&str>, bool)> {
    match rest.strip_prefix('[') {
        Some(inner) => {
            let (addr, after) = inner.split_once(']')?;
            let port = (!after.is_empty())
                .then(|| after.strip_prefix(':'))
                .flatten();
            (after.is_empty() || port.is_some()).then_some((addr, port, true))
        }
        None => Some(match rest.split_once(':') {
            Some((host, port)) => (host, Some(port), false),
            None => (rest, None, false),
        }),
    }
}

/// A lower-case host name or an IPv4 address, or the digits and colons of an IPv6 address.
fn host_ok(host: &str, v6: bool) -> bool {
    let byte_ok = |b: u8| {
        if v6 {
            b.is_ascii_hexdigit() || b == b':' || b == b'.'
        } else {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.'
        }
    };
    !host.is_empty() && host.bytes().all(byte_ok)
}

/// No port, or one of at most five digits.
fn port_ok(port: Option<&str>) -> bool {
    port.is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()))
}

/// The value of `--ws-allow-origin`: an origin as a browser sends it, `http` or `https`, a lower-case host (a name, an IPv4
/// address or a bracketed IPv6 address) and an optional port; no path and no wildcard.
pub fn parse_origin(text: &str) -> Result<String> {
    let bad = || {
        anyhow!(
            "--ws-allow-origin is an origin such as https://app.example or http://localhost:3000 (http or https, a lower-case host, an optional port; no path, no wildcard), not {:?}",
            klatch::domain::flat(text)
        )
    };
    let (scheme, rest) = text.split_once("://").ok_or_else(bad)?;
    let (host, port, v6) = authority(rest).ok_or_else(bad)?;
    let fine = matches!(scheme, "http" | "https") && host_ok(host, v6) && port_ok(port);
    fine.then(|| text.to_string()).ok_or_else(bad)
}

/// Wait for a connection to end (`ended`: its sender dropped) or for `max_age`; at the age, run `end`, which cuts the
/// connection. The client reconnects and resumes with `subscribe` and `from_seq`.
pub(crate) fn at_max_age(
    ended: &std::sync::mpsc::Receiver<()>,
    max_age: Duration,
    end: impl FnOnce(),
) {
    if let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = ended.recv_timeout(max_age) {
        end();
    }
}

// ---------------------------------------------------------------------------------------------------------------
// the registry

/// The open WebSocket connections of each member.
#[derive(Default)]
pub(crate) struct Open(Mutex<HashMap<String, usize>>);

/// One member's place among the open connections: dropping it gives the place back, on every way out of the connection.
pub(crate) struct Slot<'a> {
    open: &'a Open,
    member: String,
}

impl Open {
    /// A place for `member`, or `None` when she already holds `cap`.
    pub(crate) fn claim<'a>(&'a self, member: &str, cap: usize) -> Option<Slot<'a>> {
        let mut held = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let n = held.entry(member.to_string()).or_insert(0);
        if *n >= cap {
            return None;
        }
        *n += 1;
        Some(Slot {
            open: self,
            member: member.to_string(),
        })
    }
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        let mut held = self.open.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = held.get_mut(&self.member) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                held.remove(&self.member);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_origin_flag_takes_a_plain_origin() {
        for ok in [
            "https://app.example",
            "http://localhost:3000",
            "http://127.0.0.1:8080",
            "http://[::1]:5173",
            "https://a-b.example.org",
        ] {
            assert_eq!(parse_origin(ok).unwrap(), ok);
        }
        for bad in [
            "",
            "*",
            "app.example",
            "https://",
            "https://app.example/",
            "https://app.example/x",
            "ftp://app.example",
            "null",
            "https://*.example",
            "https://APP.example",
            "https://app.example:",
            "https://app.example:123456",
            "https://app.example:80x",
            "http://[::1",
            "http://[]:1",
            "http:// app.example",
        ] {
            assert!(parse_origin(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_member_holds_at_most_her_cap_and_a_slot_given_back_is_usable_whatever_way_it_was_given_back()
     {
        let open = Open::default();
        let a: Vec<_> = (0..2).map(|_| open.claim("alice", 2).unwrap()).collect();
        assert!(open.claim("alice", 2).is_none());
        // another member's count is her own
        let b = open.claim("bob", 2).unwrap();
        drop(a);
        assert!(open.claim("alice", 2).is_some());
        drop(b);
        // a place held by code that panics is given back by the unwind
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _slot = open.claim("carol", 1).unwrap();
            assert!(open.claim("carol", 1).is_none());
            panic!("the connection function panics");
        }));
        assert!(result.is_err());
        assert!(
            open.claim("carol", 1).is_some(),
            "the slot leaked through a panic"
        );
        assert!(
            open.0.lock().unwrap().is_empty(),
            "the registry kept an entry"
        );
    }

    fn finish(args: &[(&str, &str)], max_connections: usize, short: bool) -> Result<Options> {
        let mut raw = Raw::default();
        for (flag, value) in args {
            raw.take(flag, value)?;
        }
        raw.finish(max_connections, short)
    }

    #[test]
    fn the_cap_per_member_is_from_one_to_the_connection_cap_and_the_age_from_a_minute_to_a_day() {
        assert!(finish(&[("--ws-max-per-member", "1")], 8, false).is_ok());
        assert!(finish(&[("--ws-max-per-member", "8")], 8, false).is_ok());
        for bad in ["0", "9", "x", "", "-1", "18446744073709551615"] {
            let e = finish(&[("--ws-max-per-member", bad)], 8, false)
                .unwrap_err()
                .to_string();
            assert!(
                e.contains("--ws-max-per-member") && e.contains("--max-connections"),
                "{bad}: {e}"
            );
        }
        for ok in ["60", "3600", "86400"] {
            assert!(
                finish(&[("--ws-max-age-secs", ok)], 8, false).is_ok(),
                "{ok}"
            );
        }
        for bad in ["0", "59", "86401", "x", "", "18446744073709551615"] {
            let e = finish(&[("--ws-max-age-secs", bad)], 8, false)
                .unwrap_err()
                .to_string();
            assert!(
                e.contains("--ws-max-age-secs") && e.contains("86400"),
                "{bad}: {e}"
            );
        }
        // the test knob lowers the minimum age, and only where the build has it
        assert_eq!(
            finish(&[("--ws-max-age-secs", "1")], 8, true).is_ok(),
            cfg!(debug_assertions)
        );
        assert!(finish(&[("--ws-max-age-secs", "0")], 8, true).is_err());
    }

    #[test]
    fn the_stream_flags_have_the_bounds_of_the_websocket_flags() {
        assert!(finish(&[("--stream-max-per-member", "8")], 8, false).is_ok());
        for bad in ["0", "9", "x", ""] {
            let e = finish(&[("--stream-max-per-member", bad)], 8, false)
                .unwrap_err()
                .to_string();
            assert!(e.contains("--stream-max-per-member") && e.contains("--max-connections"));
        }
        for ok in ["60", "86400"] {
            assert!(finish(&[("--stream-max-age-secs", ok)], 8, false).is_ok());
        }
        for bad in ["0", "59", "86401", "x"] {
            let e = finish(&[("--stream-max-age-secs", bad)], 8, false)
                .unwrap_err()
                .to_string();
            assert!(e.contains("--stream-max-age-secs") && e.contains("86400"));
        }
        let o = finish(
            &[
                ("--stream-max-per-member", "2"),
                ("--stream-max-age-secs", "90"),
            ],
            8,
            false,
        )
        .unwrap();
        let mut s = Settings::default();
        s.apply(&o, 8);
        assert_eq!(
            (s.stream_per_member, s.stream_max_age),
            (2, Duration::from_secs(90))
        );
        // the WebSocket route's own values are not touched by them
        assert_eq!((s.per_member, s.max_age), (4, Duration::from_secs(3600)));
        s.apply(&Options::default(), 2);
        assert_eq!(s.stream_per_member, 2);
    }

    #[test]
    fn the_defaults_are_four_per_member_but_never_more_than_the_connection_cap_and_an_hour() {
        let mut s = Settings::default();
        s.apply(&Options::default(), 32);
        assert_eq!((s.per_member, s.max_age), (4, Duration::from_secs(3600)));
        s.apply(&Options::default(), 2);
        assert_eq!(s.per_member, 2);
    }
}
