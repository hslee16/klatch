use super::*;
use std::cell::{Cell, RefCell};
use std::time::Duration;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn headers(after: Option<&str>, after_ms: Option<&str>) -> RetryHeaders {
    RetryHeaders {
        retry_after: after.map(String::from),
        retry_after_ms: after_ms.map(String::from),
    }
}

/// A clock that moves only when the sleeper sleeps, and records every sleep. Nothing really waits.
struct World {
    now: Cell<Duration>,
    slept: RefCell<Vec<Duration>>,
    unit: Cell<f64>,
}
impl World {
    fn new(unit: f64) -> World {
        World {
            now: Cell::new(Duration::ZERO),
            slept: RefCell::new(vec![]),
            unit: Cell::new(unit),
        }
    }
}
impl Clock for World {
    fn now(&self) -> Duration {
        self.now.get()
    }
}
impl Sleeper for World {
    fn sleep(&self, d: Duration) {
        self.slept.borrow_mut().push(d);
        self.now.set(self.now.get() + d);
    }
}
impl Jitter for World {
    fn unit(&self) -> f64 {
        self.unit.get()
    }
}
fn timing(w: &World) -> Timing<'_> {
    Timing::new(w, w, w)
}

fn st(status: u16) -> JevTransportError {
    JevTransportError::status(status, "BODY-SENTINEL")
}

#[test]
fn the_defaults_mirror_the_sdk() {
    let p = RetryPolicy::default();
    assert_eq!(p.max_retries(), 2);
    assert_eq!(p.backoff_initial(), ms(500));
    assert_eq!(p.backoff_max(), ms(5000));
    assert_eq!(p.jitter(), 0.25);
    assert!(p.respects_retry_after());
    assert_eq!(p.budget(), Duration::from_secs(30));
    assert_eq!(RetryPolicy::none().max_retries(), 0);
}

#[test]
fn only_429_and_529_are_retried_never_the_rest() {
    for s in 100..600u16 {
        assert_eq!(retryable_status(s), matches!(s, 429 | 529), "{s}");
    }
    for s in [400, 401, 403, 413, 415, 422] {
        let w = World::new(0.0);
        let calls = Cell::new(0);
        let r: Result<(), _> = run_with_retry(&RetryPolicy::default(), &timing(&w), || {
            calls.set(calls.get() + 1);
            Err(st(s))
        });
        assert_eq!(r, Err(st(s)));
        assert_eq!(calls.get(), 1, "{s} must not be retried");
        assert!(w.slept.borrow().is_empty());
    }
    // a network failure is not retried either (narrower than the SDK, which retries connection errors)
    let w = World::new(0.0);
    let calls = Cell::new(0);
    let r: Result<(), _> = run_with_retry(&RetryPolicy::default(), &timing(&w), || {
        calls.set(calls.get() + 1);
        Err(JevTransportError::Network)
    });
    assert_eq!(r, Err(JevTransportError::Network));
    assert_eq!(calls.get(), 1);
}

#[test]
fn the_backoff_doubles_up_to_the_maximum() {
    let p = RetryPolicy::new(10, ms(500), ms(5000), 0.0, true, Duration::from_secs(600)).unwrap();
    let d: Vec<_> = (0..6).map(|i| p.backoff(i, 0.0)).collect();
    assert_eq!(
        d,
        vec![ms(500), ms(1000), ms(2000), ms(4000), ms(5000), ms(5000)]
    );
}

#[test]
fn the_jitter_subtracts_a_fraction_of_the_delay() {
    let p = RetryPolicy::default();
    assert_eq!(p.backoff(0, 0.0), ms(500));
    assert_eq!(p.backoff(0, 1.0), ms(375)); // 500 * (1 - 0.25)
    assert_eq!(p.backoff(0, 0.5), ms(437) + Duration::from_micros(500));
    // an out of range source is clamped, never lengthens or panics
    assert_eq!(p.backoff(0, -3.0), ms(500));
    assert_eq!(p.backoff(0, 9.0), ms(375));
    assert_eq!(p.backoff(0, f64::NAN), ms(500));
    for i in 0..40 {
        for u in [0.0, 0.3, 0.99] {
            assert!(p.backoff(i, u) <= ms(5000));
        }
    }
}

#[test]
fn a_run_retries_up_to_the_count_then_gives_the_last_error() {
    for last in [429, 529] {
        let w = World::new(0.0);
        let calls = Cell::new(0);
        let r: Result<(), _> = run_with_retry(&RetryPolicy::default(), &timing(&w), || {
            calls.set(calls.get() + 1);
            Err(st(if calls.get() == 1 { 429 } else { last }))
        });
        assert_eq!(r, Err(st(last)));
        assert_eq!(calls.get(), 3, "one try and two retries");
        assert_eq!(*w.slept.borrow(), vec![ms(500), ms(1000)]);
    }
}

#[test]
fn a_run_stops_at_the_first_success() {
    let w = World::new(1.0);
    let calls = Cell::new(0);
    let r = run_with_retry(&RetryPolicy::default(), &timing(&w), || {
        calls.set(calls.get() + 1);
        if calls.get() < 2 {
            Err(st(529))
        } else {
            Ok("fine")
        }
    });
    assert_eq!(r, Ok("fine"));
    assert_eq!(calls.get(), 2);
    assert_eq!(*w.slept.borrow(), vec![ms(375)]);
}

#[test]
fn no_retries_means_one_try() {
    let w = World::new(0.0);
    let calls = Cell::new(0);
    let r: Result<(), _> = run_with_retry(&RetryPolicy::none(), &timing(&w), || {
        calls.set(calls.get() + 1);
        Err(st(429))
    });
    assert!(r.is_err() && calls.get() == 1 && w.slept.borrow().is_empty());
}

#[test]
fn retry_after_headers_are_honoured_and_the_millisecond_one_wins() {
    let p = RetryPolicy::default();
    assert_eq!(
        p.delay(0, &headers(Some("3"), None), 0.0),
        Duration::from_secs(3)
    );
    assert_eq!(p.delay(0, &headers(Some("1.5"), None), 0.0), ms(1500));
    assert_eq!(p.delay(0, &headers(Some("3"), Some("250")), 0.0), ms(250));
    assert_eq!(p.delay(1, &headers(Some("0"), None), 0.0), Duration::ZERO);
    // not parseable, negative or not finite: the backoff is used (an HTTP date is not supported)
    for bad in [
        "soon",
        "-1",
        "NaN",
        "inf",
        "1e400",
        "Wed, 21 Oct 2026 07:28:00 GMT",
        "",
    ] {
        assert_eq!(
            p.delay(1, &headers(Some(bad), None), 0.0),
            ms(1000),
            "{bad:?}"
        );
        assert_eq!(
            p.delay(1, &headers(None, Some(bad)), 0.0),
            ms(1000),
            "{bad:?}"
        );
    }
    // a bad millisecond header falls through to a good seconds header
    assert_eq!(
        p.delay(0, &headers(Some("2"), Some("soon")), 0.0),
        Duration::from_secs(2)
    );
    // switched off, the headers are ignored
    let off = RetryPolicy::new(2, ms(500), ms(5000), 0.25, false, Duration::from_secs(30)).unwrap();
    assert_eq!(off.delay(0, &headers(Some("3"), Some("250")), 0.0), ms(500));
}

#[test]
fn a_retry_that_would_reach_the_budget_is_not_made() {
    let w = World::new(0.0);
    let calls = Cell::new(0);
    let hdr = headers(Some("40"), None); // asks for 40 s, the budget is 30 s
    let r: Result<(), _> = run_with_retry(&RetryPolicy::default(), &timing(&w), || {
        calls.set(calls.get() + 1);
        Err(JevTransportError::Status {
            status: 429,
            body: String::new(),
            headers: hdr.clone(),
        })
    });
    assert!(r.is_err());
    assert_eq!(calls.get(), 1);
    assert!(w.slept.borrow().is_empty());
    // exactly the budget is also too long ("reach or exceed")
    let w = World::new(0.0);
    let hdr = headers(Some("30"), None);
    let calls = Cell::new(0);
    let _: Result<(), _> = run_with_retry(&RetryPolicy::default(), &timing(&w), || {
        calls.set(calls.get() + 1);
        Err(JevTransportError::Status {
            status: 529,
            body: String::new(),
            headers: hdr.clone(),
        })
    });
    assert_eq!(calls.get(), 1);
    // under the budget it is made, and the time already spent counts
    let w = World::new(0.0);
    let hdr = headers(Some("20"), None);
    let calls = Cell::new(0);
    let _: Result<(), _> = run_with_retry(&RetryPolicy::default(), &timing(&w), || {
        calls.set(calls.get() + 1);
        Err(JevTransportError::Status {
            status: 529,
            body: String::new(),
            headers: hdr.clone(),
        })
    });
    assert_eq!(
        calls.get(),
        2,
        "20 s spent, the second 20 s would pass 30 s"
    );
    assert_eq!(*w.slept.borrow(), vec![Duration::from_secs(20)]);
}

#[test]
fn the_settings_are_validated() {
    let ok =
        |r: u32, i: u64, m: u64, j: f64, b: u64| RetryPolicy::new(r, ms(i), ms(m), j, true, ms(b));
    assert!(ok(2, 500, 5000, 0.25, 30_000).is_ok());
    assert!(
        ok(11, 500, 5000, 0.25, 30_000).is_err(),
        "more than 10 retries"
    );
    assert!(
        ok(2, 6000, 5000, 0.25, 30_000).is_err(),
        "initial above max"
    );
    assert!(ok(2, 500, 5000, 1.5, 30_000).is_err());
    assert!(ok(2, 500, 5000, -0.1, 30_000).is_err());
    assert!(ok(2, 500, 5000, f64::NAN, 30_000).is_err());
    assert!(
        ok(2, 500, 5000, 0.25, 0).is_err(),
        "no budget is no retry, use none()"
    );
    assert!(ok(0, 500, 5000, 0.0, 1).is_ok());
}

#[test]
fn the_system_clock_and_sleeper_work_without_a_real_wait() {
    let c = SystemClock::new();
    let a = c.now();
    ThreadSleeper.sleep(Duration::ZERO);
    assert!(c.now() >= a);
    assert_eq!(NoJitter.unit(), 0.0);
}
