//! Showing and keeping the ignored forwarded-header counts (#1490, epic #13). `forwarded.rs` counts, in memory, by header
//! name only. This module is the timer that reports those counts:
//!
//! - **live read**: while the server runs, one counts-only line on stderr, at most one per interval and only when the
//!   counts changed since the last line (`klatch-server: forwarded identity headers seen so far: requests=N name=K ...`);
//! - **durable**: one audit row (`forwarded-ignored`, `Backend::note_forwarded_ignored`) per flush interval, only when
//!   the counts changed, holding what was counted since the last row; plus one at shutdown. `klatch admin audit` shows
//!   the rows, and summing them gives the total across restarts.
//!
//! Nothing here runs on a request. A request only bumps an in-memory counter; the timer thread is the only writer, so
//! a flood of requests with these headers cannot cause more lines or rows than one per interval (and one at stop). The
//! text handed out is `Counts::render`: names of the fixed list and numbers, never a header value (this module is never
//! handed one).

use crate::forwarded::{Counts, Ignored};
use klatch::session::Backend;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How often the timer thread looks. Only a look: whether anything is written is `Pacer`'s rule.
const TICK: Duration = Duration::from_millis(50);

/// The intervals: a stderr line at most this often, an audit row at most this often.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Schedule {
    pub(crate) line: Duration,
    pub(crate) flush: Duration,
}

impl Schedule {
    /// The shipped intervals: a line a minute, a row every ten minutes.
    pub(crate) const SHIPPED: Schedule = Schedule {
        line: Duration::from_secs(60),
        flush: Duration::from_secs(600),
    };
    /// What a debug build takes with `KLATCH_TEST_SHORT_LIMITS` (the knob the other short limits use): a release binary
    /// has no such knob.
    const SHORT: Schedule = Schedule {
        line: Duration::from_secs(1),
        flush: Duration::from_secs(2),
    };

    pub(crate) fn current() -> Schedule {
        if crate::wire_ws_caps::short_limits() {
            Schedule::SHORT
        } else {
            Schedule::SHIPPED
        }
    }
}

/// What to do now: text for the stderr line and text for the audit row, each only when due.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Due {
    pub(crate) line: Option<String>,
    pub(crate) flush: Option<String>,
}

/// The rate rules, with no clock and no output of their own (the caller passes `now`).
pub(crate) struct Pacer {
    sched: Schedule,
    next_line: Instant,
    next_flush: Instant,
    line_shown: Counts,
    flushed: Counts,
}

impl Pacer {
    pub(crate) fn new(now: Instant, sched: Schedule) -> Pacer {
        Pacer {
            sched,
            next_line: now + sched.line,
            next_flush: now + sched.flush,
            line_shown: Counts::default(),
            flushed: Counts::default(),
        }
    }

    /// What is due at `now` given the counts. A line is due once its interval has passed and the counts differ from the
    /// last line shown; a row likewise against the last row kept. The interval restarts when one is handed out, so
    /// nothing is due again before it passes, however the counts move.
    pub(crate) fn due(&mut self, now: Instant, counts: &Counts) -> Due {
        let mut due = Due::default();
        if now >= self.next_line && *counts != self.line_shown {
            self.next_line = now + self.sched.line;
            self.line_shown = counts.clone();
            due.line = Some(format!("{LINE_PREFIX}{}", counts.render()));
        }
        if now >= self.next_flush && !counts.since(&self.flushed).is_zero() {
            // restarts whether or not the row is kept: a store that refuses is not asked at every look
            self.next_flush = now + self.sched.flush;
            due.flush = Some(counts.since(&self.flushed).render());
        }
        due
    }

    /// The row handed out by `due` was kept: the next row holds what is counted after `counts`.
    pub(crate) fn kept(&mut self, counts: &Counts) {
        self.flushed = counts.clone();
    }

    /// The row for shutdown, whatever the timer says: what is counted and not yet kept.
    pub(crate) fn last(&self, counts: &Counts) -> Option<String> {
        let rest = counts.since(&self.flushed);
        (!rest.is_zero()).then(|| rest.render())
    }
}

/// Sets a flag when dropped, so the timer thread ends however `serve` ends (a panic included).
struct EndOnDrop<'a>(&'a AtomicBool);

impl Drop for EndOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// The timer thread: look every `TICK`, write what the pacer says is due. It returns its pacer so that shutdown can
/// keep what is left. The only writer of the live line and of the periodic rows.
fn timer(ignored: &Ignored, backend: &dyn Backend, done: &AtomicBool, mut pacer: Pacer) -> Pacer {
    while !done.load(Ordering::SeqCst) {
        std::thread::sleep(TICK);
        let counts = ignored.snapshot();
        let due = pacer.due(Instant::now(), &counts);
        if let Some(line) = due.line {
            eprintln!("{line}");
        }
        if let Some(text) = due.flush
            && backend.note_forwarded_ignored(&text).is_ok()
        {
            pacer.kept(&counts);
        }
    }
    pacer
}

/// Run `serve` (the accept loop, until the server is told to stop) with the timer beside it. When `serve` returns, every
/// connection is done: the last row is kept and the shutdown summary written, once. Best effort like every audit row:
/// a row that cannot be kept never changes what the server did.
pub(crate) fn while_reporting<R>(
    ignored: &Ignored,
    backend: &(dyn Backend + Sync),
    serve: impl FnOnce() -> R,
) -> R {
    let done = AtomicBool::new(false);
    let pacer = Pacer::new(Instant::now(), Schedule::current());
    let (out, pacer) = std::thread::scope(|scope| {
        let timer = scope.spawn(|| timer(ignored, backend, &done, pacer));
        let out = {
            let _end = EndOnDrop(&done);
            serve()
        };
        (out, timer.join())
    });
    let counts = ignored.snapshot();
    if let Ok(pacer) = pacer
        && let Some(text) = pacer.last(&counts)
    {
        let _ = backend.note_forwarded_ignored(&text);
    }
    eprintln!("{}", ignored.summary());
    out
}

/// The start of the live line. Not the words of the shutdown line (`forwarded identity headers ignored`), so a reader
/// can tell the two apart.
const LINE_PREFIX: &str = "klatch-server: forwarded identity headers seen so far: ";

#[cfg(test)]
mod tests {
    use super::*;

    const S: Schedule = Schedule {
        line: Duration::from_secs(60),
        flush: Duration::from_secs(600),
    };

    fn at(base: Instant, secs: u64) -> Instant {
        base + Duration::from_secs(secs)
    }

    #[test]
    fn nothing_is_due_while_nothing_is_counted() {
        let t0 = Instant::now();
        let mut p = Pacer::new(t0, S);
        for secs in [0, 60, 600, 6000] {
            assert_eq!(p.due(at(t0, secs), &Counts::default()), Due::default());
        }
        assert_eq!(p.last(&Counts::default()), None);
    }

    #[test]
    fn a_line_is_due_only_after_its_interval_and_only_when_the_counts_changed() {
        let t0 = Instant::now();
        let mut p = Pacer::new(t0, S);
        let c = Counts::of(3);
        assert_eq!(p.due(at(t0, 59), &c).line, None, "before the interval");
        let line = p.due(at(t0, 60), &c).line.expect("due at the interval");
        assert!(line.ends_with("requests=3 forwarded=3"), "{line}");
        assert!(line.starts_with("klatch-server: forwarded identity headers seen so far: "));
        assert_eq!(p.due(at(t0, 120), &c).line, None, "unchanged: no line");
        assert!(p.due(at(t0, 180), &Counts::of(4)).line.is_some());
    }

    #[test]
    fn a_flood_gets_one_line_and_one_row_per_interval_not_one_per_request() {
        let t0 = Instant::now();
        let mut p = Pacer::new(t0, S);
        let (mut lines, mut rows) = (0, 0);
        // the counts rise at every look for an hour; the pacer is looked at every 50 ms
        for i in 0..=72_000u64 {
            let now = t0 + Duration::from_millis(50 * i);
            let due = p.due(now, &Counts::of(i + 1));
            lines += usize::from(due.line.is_some());
            if due.flush.is_some() {
                rows += 1;
                p.kept(&Counts::of(i + 1));
            }
        }
        assert_eq!(lines, 60, "one line a minute for an hour");
        assert_eq!(rows, 6, "one row every ten minutes for an hour");
    }

    #[test]
    fn a_row_holds_what_was_counted_since_the_last_row() {
        let t0 = Instant::now();
        let mut p = Pacer::new(t0, S);
        let first = p.due(at(t0, 600), &Counts::of(5)).flush.expect("a row");
        assert_eq!(first, "requests=5 forwarded=5");
        p.kept(&Counts::of(5));
        assert_eq!(p.due(at(t0, 1200), &Counts::of(5)).flush, None);
        let second = p.due(at(t0, 1800), &Counts::of(7)).flush.expect("a row");
        assert_eq!(second, "requests=2 forwarded=2");
    }

    #[test]
    fn a_row_that_could_not_be_kept_is_tried_again_at_the_next_interval_not_at_every_look() {
        let t0 = Instant::now();
        let mut p = Pacer::new(t0, S);
        let c = Counts::of(5);
        assert!(p.due(at(t0, 600), &c).flush.is_some());
        // not kept: no `kept` call
        assert_eq!(p.due(at(t0, 601), &c).flush, None, "not again at once");
        let again = p.due(at(t0, 1200), &c).flush.expect("tried again");
        assert_eq!(again, "requests=5 forwarded=5", "nothing was lost");
    }

    #[test]
    fn shutdown_gives_what_is_unkept_whatever_the_timer_says_and_nothing_when_all_is_kept() {
        let t0 = Instant::now();
        let mut p = Pacer::new(t0, S);
        assert_eq!(
            p.last(&Counts::of(2)).as_deref(),
            Some("requests=2 forwarded=2")
        );
        p.kept(&Counts::of(2));
        assert_eq!(p.last(&Counts::of(2)), None);
        assert_eq!(
            p.last(&Counts::of(3)).as_deref(),
            Some("requests=1 forwarded=1")
        );
    }

    #[test]
    fn what_is_handed_out_is_names_and_numbers_only() {
        let text = Counts::of(9).render();
        assert!(
            text.bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'=' | b'-' | b'*' | b' ')),
            "{text}"
        );
    }
}
