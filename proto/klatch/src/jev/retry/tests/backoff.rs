use super::*;

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
