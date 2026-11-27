//! Tests of the TLS reload (#1492, #1701): the reload decision, the failure lines, the stuck-build bounds.

use super::*;
use std::path::PathBuf;

fn files() -> TlsFiles {
    TlsFiles {
        cert: "/d/client CA file/c.pem".into(),
        key: PathBuf::from("/d/k.pem"),
        client_ca: PathBuf::from("/d/ca.pem"),
        client_crl: Some(PathBuf::from("/d/crl.pem")),
        client_intermediates: None,
    }
}

#[test]
fn a_failure_is_classified_by_its_words_and_not_by_a_directory_name() {
    let f = files();
    let e = |m: &str| anyhow::anyhow!("{m}");
    let cert = e("cannot read the server certificate /d/client CA file/c.pem: NotFound");
    assert_eq!(classify(&cert, &f), Kind::Certificate);
    let key = e("the key file /d/k.pem must be mode 0600");
    assert_eq!(classify(&key, &f), Kind::KeyFile);
    let ca = e("the client CA file /d/ca.pem holds no certificate");
    assert_eq!(classify(&ca, &f), Kind::CaFile);
    let crl = e("the client CRL file /d/crl.pem is empty");
    assert_eq!(classify(&crl, &f), Kind::CrlFile);
    let fit =
        e("the server certificate and key do not fit together, or the certificate is unusable");
    assert_eq!(classify(&fit, &f), Kind::NoFit);
    assert_eq!(classify(&e("something new"), &f), Kind::Other);
}

#[test]
fn every_line_is_a_fixed_template_with_no_file_name() {
    let f = files();
    for kind in [
        Kind::CaFile,
        Kind::CrlFile,
        Kind::KeyFile,
        Kind::Certificate,
        Kind::NoFit,
        Kind::Other,
        Kind::TimedOut,
        Kind::Busy,
        Kind::Restart,
    ] {
        let line = failure_line(kind);
        assert!(!line.contains("/d/"), "{line}");
        assert!(line.starts_with("klatch-server: https: reload failed: "));
    }
    assert!(!success_line(&f).contains("/d/"));
}

#[test]
fn a_swap_replaces_the_value_new_readers_get_and_leaves_the_old_one_alive_for_its_holders() {
    let (a, b) = (Arc::new(1), Arc::new(2));
    let current = Current::new(a.clone());
    let held = current.get();
    current.swap(b.clone());
    assert!(Arc::ptr_eq(&current.get(), &b));
    assert!(Arc::ptr_eq(&held, &a));
}

#[test]
fn a_wait_for_a_build_ends_at_the_deadline_on_stop_and_when_the_builder_is_gone() {
    let stop = AtomicBool::new(false);
    let (tx, rx) = channel::<u8>();
    assert_eq!(
        wait_for(&rx, &stop, Duration::from_millis(150)),
        Wait::TimedOut
    );
    tx.send(7).unwrap();
    assert_eq!(wait_for(&rx, &stop, Duration::from_secs(5)), Wait::Got(7));
    // stopping ends a long wait within a tick, not at the deadline
    let started = Instant::now();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(200));
            stop.store(true, Ordering::SeqCst);
        });
        assert_eq!(wait_for(&rx, &stop, Duration::from_secs(30)), Wait::Stopped);
    });
    assert!(started.elapsed() < Duration::from_secs(5));
    drop(tx);
    stop.store(false, Ordering::SeqCst);
    assert_eq!(wait_for(&rx, &stop, Duration::from_secs(5)), Wait::Gone);
}

/// Make the build in flight look `by` older: a bound is crossed without waiting (no timing margin to flake).
fn backdate(slot: &mut Slot, by: Duration) {
    slot.started = slot
        .started
        .checked_sub(by)
        .expect("the clock is past boot");
}

/// A `Slot` for the tests: a build that blocks until the test lets it go, and one that answers at once.
fn blocked_build<T: Send + 'static>(
    release: Receiver<()>,
    value: T,
) -> impl FnOnce() -> Result<(Arc<T>, Option<i64>), Kind> + Send + 'static {
    move || {
        let _ = release.recv();
        Ok((Arc::new(value), None))
    }
}

/// #1637: a build blocked past [`DEADLINE`] on a stalled mount disables reload only until the abandon bound, after
/// which the next reload starts; the abandoned build, when it finally ends, can never swap its configuration over
/// the newer one (its answer has no receiver) nor clear the flag of the build that replaced it.
#[test]
fn a_build_stuck_past_the_bound_is_abandoned_and_its_late_answer_never_applies() {
    let stop = AtomicBool::new(false);
    let current = Current::new(Arc::new(0u8));
    let mut slot = Slot::new();
    let mut due = None;
    let limits = Limits {
        deadline: Duration::from_millis(100),
        abandon_after: Duration::from_secs(30),
        max_abandoned: 2,
    };
    let (free_first, hold_first) = channel();
    let first = reload_with(
        blocked_build(hold_first, 1u8),
        "ok",
        &current,
        &mut slot,
        &stop,
        &mut due,
        &limits,
    );
    assert_eq!(first, Some(failure_line(Kind::TimedOut)));
    let early = reload_with(
        || Ok((Arc::new(9u8), None)),
        "ok",
        &current,
        &mut slot,
        &stop,
        &mut due,
        &limits,
    );
    assert_eq!(early, Some(failure_line(Kind::Busy)));
    assert_eq!(*current.get(), 0);
    // past the bound the stuck build is given up, said in the log, and a new reload runs and swaps
    backdate(&mut slot, Duration::from_secs(31));
    let later = reload_with(
        || Ok((Arc::new(2u8), None)),
        "ok",
        &current,
        &mut slot,
        &stop,
        &mut due,
        &limits,
    )
    .unwrap();
    assert!(
        later.contains("abandoned") && later.contains("restart"),
        "{later}"
    );
    assert!(later.ends_with("ok"), "{later}");
    assert_eq!(*current.get(), 2);
    // the build thread clears its flag just after it answers: wait for that
    let waited = Instant::now();
    while slot.busy.load(Ordering::SeqCst) {
        assert!(
            waited.elapsed() < Duration::from_secs(10),
            "the busy flag never cleared"
        );
        std::thread::yield_now();
    }
    // the abandoned build ends now: nothing changes, and the flag of a running build is not cleared by it
    let (free_third, hold_third) = channel();
    let third = reload_with(
        blocked_build(hold_third, 3u8),
        "ok",
        &current,
        &mut slot,
        &stop,
        &mut due,
        &limits,
    );
    assert_eq!(third, Some(failure_line(Kind::TimedOut)));
    free_first.send(()).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(*current.get(), 2, "a late answer was applied");
    let busy = reload_with(
        || Ok((Arc::new(8u8), None)),
        "ok",
        &current,
        &mut slot,
        &stop,
        &mut due,
        &limits,
    );
    assert_eq!(
        busy,
        Some(failure_line(Kind::Busy)),
        "the abandoned build cleared the flag of the build that replaced it"
    );
    free_third.send(()).unwrap();
}

/// The number of abandoned builds is bounded: a mount that stays stalled costs a few threads, not one per reload.
#[test]
fn abandoning_stops_at_the_cap_and_then_the_flag_stays() {
    let stop = AtomicBool::new(false);
    let current = Current::new(Arc::new(0u8));
    let mut slot = Slot::new();
    let mut due = None;
    let limits = Limits {
        deadline: Duration::from_millis(50),
        abandon_after: Duration::from_secs(30),
        max_abandoned: 1,
    };
    let mut releases = Vec::new();
    let mut lines = Vec::new();
    for _ in 0..3 {
        let (free, hold) = channel();
        releases.push(free);
        lines.push(reload_with(
            blocked_build(hold, 1u8),
            "ok",
            &current,
            &mut slot,
            &stop,
            &mut due,
            &limits,
        ));
        backdate(&mut slot, Duration::from_secs(31));
    }
    assert_eq!(lines[0], Some(failure_line(Kind::TimedOut)));
    assert!(
        lines[1].as_deref().unwrap().contains("abandoned"),
        "{:?}",
        lines[1]
    );
    // the third finds the cap reached: only a restart clears the stuck build (#1694)
    assert!(
        lines[2].as_deref().unwrap().contains("restart the server"),
        "{:?}",
        lines[2]
    );
    for free in releases {
        let _ = free.send(());
    }
}

/// #1694: the cap means "abandoned and still stuck": when an abandoned build ends, its place is given back, so a later
/// stall can be abandoned again; the count never goes below zero and a build that was never abandoned changes nothing.
#[test]
fn an_abandoned_build_that_finishes_gives_its_place_back_to_the_cap() {
    let stop = AtomicBool::new(false);
    let current = Current::new(Arc::new(0u8));
    let mut slot = Slot::new();
    let mut due = None;
    let limits = Limits {
        deadline: Duration::from_millis(50),
        abandon_after: Duration::from_secs(30),
        max_abandoned: 1,
    };
    let start = |slot: &mut Slot, due: &mut Option<i64>| {
        let (free, hold) = channel();
        let line = reload_with(
            blocked_build(hold, 1u8),
            "ok",
            &current,
            slot,
            &stop,
            due,
            &limits,
        );
        (free, line)
    };
    let (free_first, _) = start(&mut slot, &mut due);
    backdate(&mut slot, Duration::from_secs(31));
    let (free_second, second) = start(&mut slot, &mut due);
    assert!(second.unwrap().contains("abandoned"));
    assert_eq!(slot.abandoned_now(), 1);
    // at the cap: the next stuck build cannot be abandoned
    backdate(&mut slot, Duration::from_secs(31));
    let (free_capped, capped) = start(&mut slot, &mut due);
    assert!(capped.unwrap().contains("restart the server"));
    // the first (abandoned) build ends: its place is free again
    free_first.send(()).unwrap();
    let waited = Instant::now();
    while slot.abandoned_now() != 0 {
        assert!(
            waited.elapsed() < Duration::from_secs(10),
            "never given back"
        );
        std::thread::yield_now();
    }
    backdate(&mut slot, Duration::from_secs(31));
    let (free_fourth, fourth) = start(&mut slot, &mut due);
    assert!(fourth.unwrap().contains("abandoned"));
    assert_eq!(slot.abandoned_now(), 1);
    // the build in flight finishing normally never lowers the count: wait for the end of its guard (the marker is set
    // there, after the flag), then a moment more for a wrong give-back to show (a wrong one can only be missed by being slow)
    free_fourth.send(()).unwrap();
    wait_until("the build never ended", || {
        !slot.busy.load(Ordering::SeqCst) && slot.ended.load(Ordering::SeqCst)
    });
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(slot.abandoned_now(), 1);
    for free in [free_second, free_capped] {
        let _ = free.send(());
    }
}

/// Wait (bounded) until `done` holds: the build thread clears its flag and gives its place back just after it answers.
fn wait_until(what: &str, done: impl Fn() -> bool) {
    let waited = Instant::now();
    while !done() {
        assert!(waited.elapsed() < Duration::from_secs(10), "{what}");
        std::thread::yield_now();
    }
}

/// #1714 review: a mount that stalls now and then (a good reload, then a hung one, again and again) must engage the cap,
/// say "abandoned", and keep the count exact. A marker reused from a build that ended normally made the next stuck build
/// uncounted and unlogged, and its end then took a place from another stuck build.
#[test]
fn a_good_reload_then_a_stall_repeated_is_counted_logged_and_capped() {
    let stop = AtomicBool::new(false);
    let current = Current::new(Arc::new(0u8));
    let mut slot = Slot::new();
    let mut due = None;
    let limits = Limits {
        deadline: Duration::from_millis(50),
        abandon_after: Duration::from_secs(30),
        max_abandoned: 2,
    };
    let mut releases = Vec::new();
    let mut lines = Vec::new();
    for round in 0..4 {
        let good = reload_with(
            || Ok((Arc::new(5u8), None)),
            "ok",
            &current,
            &mut slot,
            &stop,
            &mut due,
            &limits,
        );
        lines.push(good.unwrap());
        if round == 3 {
            break;
        }
        wait_until("the good build never ended", || {
            !slot.busy.load(Ordering::SeqCst)
        });
        let (free, hold) = channel();
        releases.push(free);
        let stalled = reload_with(
            blocked_build(hold, 1u8),
            "ok",
            &current,
            &mut slot,
            &stop,
            &mut due,
            &limits,
        );
        assert_eq!(stalled, Some(failure_line(Kind::TimedOut)));
        backdate(&mut slot, Duration::from_secs(31));
    }
    // round 0: nothing stuck before it; rounds 1 and 2 each abandon the stall before them; round 3 finds the cap reached
    assert!(!lines[0].contains("abandoned"), "{:?}", lines[0]);
    for n in [1, 2] {
        assert!(lines[n].contains("abandoned"), "round {n}: {:?}", lines[n]);
        assert_eq!(*current.get(), 5);
    }
    assert_eq!(slot.abandoned_now(), 2);
    assert!(lines[3].contains("restart the server"), "{:?}", lines[3]);
    // the stuck builds end one by one: the count falls to zero and stays there
    for free in releases {
        let _ = free.send(());
    }
    wait_until("the places were not all given back", || {
        slot.abandoned_now() == 0
    });
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(slot.abandoned_now(), 0);
}

/// A build that ends in the very moment it is abandoned was never stuck: the count is taken back and nothing is logged as
/// abandoned. (The interleaving is set up by hand: the guard has set the marker and cleared the flag's successor.)
#[test]
fn a_build_that_ended_while_being_abandoned_is_not_counted() {
    let mut slot = Slot::new();
    slot.busy.store(true, Ordering::SeqCst);
    backdate(&mut slot, Duration::from_secs(31));
    slot.ended.store(true, Ordering::SeqCst);
    let (gave_up, _guard) = slot
        .claim(Duration::from_secs(30), 3)
        .unwrap_or_else(|restart| panic!("the claim was refused (restart: {restart})"));
    assert!(!gave_up);
    assert_eq!(slot.abandoned_now(), 0);
}

/// The count never goes below zero, however often a place is given back.
#[test]
fn giving_a_place_back_never_goes_below_zero() {
    let count = AtomicUsize::new(0);
    give_back(&count);
    give_back(&count);
    assert_eq!(count.load(Ordering::SeqCst), 0);
    let count = AtomicUsize::new(2);
    give_back(&count);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
