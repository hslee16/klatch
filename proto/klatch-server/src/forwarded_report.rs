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
    /// A refusal was said and no row has been kept since.
    refusal_said: bool,
}

impl Pacer {
    pub(crate) fn new(now: Instant, sched: Schedule) -> Pacer {
        Pacer {
            sched,
            next_line: now + sched.line,
            next_flush: now + sched.flush,
            line_shown: Counts::default(),
            flushed: Counts::default(),
            refusal_said: false,
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
        self.refusal_said = false;
    }

    /// The row handed out was refused: the one fixed line that says so, the first time and again only after a row was
    /// kept; `None` while it keeps failing. It names no error text (a store error may carry a path).
    pub(crate) fn refused(&mut self) -> Option<String> {
        (!std::mem::replace(&mut self.refusal_said, true)).then(|| REFUSED_LINE.to_string())
    }

    /// The row for shutdown, whatever the timer says: what is counted and not yet kept.
    pub(crate) fn last(&self, counts: &Counts) -> Option<String> {
        let rest = counts.since(&self.flushed);
        (!rest.is_zero()).then(|| rest.render())
    }
}

/// Where a row goes: `true` when it was kept. The server's sink is the backend's audit; a test's may refuse.
pub(crate) trait RowSink: Sync {
    fn keep(&self, text: &str) -> bool;
}

/// The audit of the gateway the server embeds.
struct Audit<'a>(&'a (dyn Backend + Sync));

impl RowSink for Audit<'_> {
    fn keep(&self, text: &str) -> bool {
        self.0.note_forwarded_ignored(text).is_ok()
    }
}

/// Sets a flag when dropped, so the timer thread ends however `serve` ends (a panic included).
struct EndOnDrop<'a>(&'a AtomicBool);

impl Drop for EndOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// One line to stderr. A write that fails (a closed stderr) is dropped: it must not end the timer, whose periodic rows
/// do not depend on the terminal (`eprintln!` panics on a failed write).
fn say(line: &str) {
    say_to(&mut std::io::stderr().lock(), line);
}

fn say_to(out: &mut dyn std::io::Write, line: &str) {
    let _ = writeln!(out, "{line}");
}

/// The timer thread: look every `TICK`, write what the pacer says is due. It returns its pacer so that shutdown can
/// keep what is left. The only writer of the live line and of the periodic rows.
fn timer(ignored: &Ignored, sink: &dyn RowSink, done: &AtomicBool, mut pacer: Pacer) -> Pacer {
    while !done.load(Ordering::SeqCst) {
        std::thread::sleep(TICK);
        let counts = ignored.snapshot();
        let due = pacer.due(Instant::now(), &counts);
        if let Some(line) = due.line {
            say(&line);
        }
        if let Some(text) = due.flush {
            keep_or_say(&mut pacer, sink, (&text, &counts));
        }
    }
    pacer
}

/// Offer one row to the sink: kept, the pacer moves on; refused, one fixed line says so (the first time, and again
/// only after a row was kept) and the counts stay unkept for the next row.
fn keep_or_say(pacer: &mut Pacer, sink: &dyn RowSink, (text, counts): (&str, &Counts)) {
    if sink.keep(text) {
        pacer.kept(counts);
    } else if let Some(line) = pacer.refused() {
        say(&line);
    }
}

/// Run `serve` (the accept loop, until the server is told to stop) with the timer beside it. When `serve` returns, every
/// connection is done: the last row is kept and the shutdown summary written, once. Best effort like every audit row:
/// a row that cannot be kept never changes what the server did.
pub(crate) fn while_reporting<R>(
    ignored: &Ignored,
    backend: &(dyn Backend + Sync),
    serve: impl FnOnce() -> R,
) -> R {
    while_reporting_with(ignored, &Audit(backend), Schedule::current(), serve)
}

/// `while_reporting` with the sink and the intervals given.
fn while_reporting_with<R>(
    ignored: &Ignored,
    sink: &dyn RowSink,
    sched: Schedule,
    serve: impl FnOnce() -> R,
) -> R {
    let done = AtomicBool::new(false);
    let pacer = Pacer::new(Instant::now(), sched);
    let (out, pacer) = std::thread::scope(|scope| {
        let timer = scope.spawn(|| timer(ignored, sink, &done, pacer));
        let out = {
            let _end = EndOnDrop(&done);
            serve()
        };
        (out, timer.join())
    });
    let counts = ignored.snapshot();
    if let Ok(mut pacer) = pacer
        && let Some(text) = pacer.last(&counts)
    {
        keep_or_say(&mut pacer, sink, (&text, &counts));
    }
    say(&ignored.summary());
    out
}

/// What is said when the store refuses a row. Fixed text.
const REFUSED_LINE: &str = "klatch-server: forwarded identity headers: the audit row was refused; the counts stay in memory and the row is tried again at the next interval";

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

    // ---- the timer thread, with a store that can refuse and a serve that can panic

    use std::sync::Mutex;

    /// A sink that refuses its first `fail_first` rows and keeps a record of every row it was offered.
    struct Sink {
        fail_first: usize,
        offered: Mutex<Vec<(String, bool)>>,
    }

    impl Sink {
        fn new(fail_first: usize) -> Sink {
            Sink {
                fail_first,
                offered: Mutex::new(Vec::new()),
            }
        }
        fn offered(&self) -> Vec<(String, bool)> {
            self.offered.lock().unwrap().clone()
        }
    }

    impl RowSink for Sink {
        fn keep(&self, text: &str) -> bool {
            let mut offered = self.offered.lock().unwrap();
            let ok = offered.len() >= self.fail_first;
            offered.push((text.to_string(), ok));
            ok
        }
    }

    fn quick() -> Schedule {
        Schedule {
            line: Duration::from_millis(40),
            flush: Duration::from_millis(100),
        }
    }

    /// Run `body` on a thread and fail the test, not hang it, if it has not finished within ten seconds.
    fn within_ten_seconds(body: impl FnOnce() + Send + 'static) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            body();
            let _ = tx.send(());
        });
        rx.recv_timeout(Duration::from_secs(10))
            .expect("hung: the timer did not stop");
    }

    #[test]
    fn a_row_the_store_refuses_keeps_its_counts_for_the_next_row() {
        within_ten_seconds(|| {
            let ignored = Ignored::default();
            ignored.bump(3);
            let sink = Sink::new(1);
            while_reporting_with(&ignored, &sink, quick(), || {
                let until = Instant::now() + Duration::from_secs(8);
                while sink.offered().len() < 2 && Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(10));
                }
            });
            let row = "requests=3 forwarded=3".to_string();
            assert_eq!(sink.offered(), [(row.clone(), false), (row, true)]);
        });
    }

    #[test]
    fn shutdown_keeps_what_the_timer_had_not_and_a_refused_shutdown_row_is_offered_once() {
        within_ten_seconds(|| {
            let ignored = Ignored::default();
            let sink = Sink::new(0);
            while_reporting_with(&ignored, &sink, quick(), || ignored.bump(2));
            assert_eq!(
                sink.offered(),
                [("requests=2 forwarded=2".to_string(), true)]
            );
            let none = Sink::new(0);
            while_reporting_with(&Ignored::default(), &none, quick(), || ());
            assert!(none.offered().is_empty(), "nothing counted, nothing kept");
        });
    }

    #[test]
    fn a_panicking_serve_stops_the_timer_and_the_shutdown_does_not_hang() {
        within_ten_seconds(|| {
            let ignored = Ignored::default();
            let sink = Sink::new(0);
            let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                while_reporting_with(&ignored, &sink, quick(), || -> () {
                    panic!("serve failed")
                })
            }));
            assert!(out.is_err(), "the panic goes on");
        });
    }

    #[test]
    fn a_refused_row_is_said_once_and_again_only_after_a_row_was_kept() {
        let t0 = Instant::now();
        let mut p = Pacer::new(t0, S);
        let said = p.refused().expect("the first refusal is said");
        assert_eq!(
            said,
            "klatch-server: forwarded identity headers: the audit row was refused; the counts stay in memory and the row is tried again at the next interval"
        );
        assert_eq!(p.refused(), None, "not again while it keeps failing");
        assert_eq!(p.refused(), None);
        p.kept(&Counts::of(1));
        assert_eq!(
            p.refused().as_deref(),
            Some(said.as_str()),
            "said again after a success"
        );
    }

    // ---- a closed stderr, and a timer that does not spin

    /// A writer whose every write fails, as a closed stderr does.
    struct Closed;

    impl std::io::Write for Closed {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
        }
    }

    #[test]
    fn saying_a_line_to_a_closed_stream_does_not_panic_and_a_good_one_gets_the_line() {
        say_to(&mut Closed, "a line");
        let mut out = Vec::new();
        say_to(&mut out, "a line");
        assert_eq!(out, b"a line\n");
    }

    #[test]
    fn the_timer_looks_at_the_counts_a_few_times_a_second_not_in_a_spin() {
        within_ten_seconds(|| {
            let ignored = Ignored::default();
            let sink = Sink::new(0);
            while_reporting_with(&ignored, &sink, quick(), || {
                std::thread::sleep(Duration::from_millis(500));
            });
            // one look every 50 ms is 10 in half a second; without the sleep it is millions
            let looks = ignored.looks();
            assert!((1..=40).contains(&looks), "{looks} looks in half a second");
        });
    }
}
