use super::*;

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
