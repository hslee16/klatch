//! Boundaries of the claim and lease code that the topic tests leave open (#725): the release note limit, the policy
//! values that count as a setting, the cap and its default in seconds, and the clock handed to the probe.

use super::*;
use crate::gateway::gateway::lease::{
    DEFAULT_CAP_SECS, DEFAULT_EXTEND_SECS, DEFAULT_GRACE_SECS, LeaseSource, Observed,
};
use crate::harness::Verdict;
use std::cell::RefCell;

fn note_result(len: usize) -> Result<()> {
    let (g, _) = timed();
    g.claim(&alice(), "r", &ClaimSpec::new("p", 100, ""))
        .unwrap();
    g.release_with_note(&alice(), ("r", "p"), &"n".repeat(len))
}

#[test]
fn a_release_note_of_300_characters_is_kept_and_one_more_is_refused() {
    assert!(note_result(300).is_ok());
    assert!(is_invalid(note_result(301)));
    assert!(is_invalid(note_result(302)));
    assert!(is_invalid(note_result(5_000)));
    assert!(note_result(1).is_ok());
}

#[test]
fn the_limit_counts_characters_not_bytes() {
    let (g, _) = timed();
    g.claim(&alice(), "r", &ClaimSpec::new("p", 100, ""))
        .unwrap();
    // 300 two-byte characters are 600 bytes
    g.release_with_note(&alice(), ("r", "p"), &"é".repeat(300))
        .unwrap();
    g.claim(&alice(), "r", &ClaimSpec::new("p", 100, ""))
        .unwrap();
    assert!(is_invalid(g.release_with_note(
        &alice(),
        ("r", "p"),
        &"é".repeat(301)
    )));
}

#[test]
fn the_default_lease_times_are_ten_minutes_five_minutes_and_four_hours() {
    assert_eq!(DEFAULT_EXTEND_SECS, 600);
    assert_eq!(DEFAULT_GRACE_SECS, 300);
    assert_eq!(DEFAULT_CAP_SECS, 14_400);
}

#[test]
fn a_zero_negative_or_unreadable_setting_is_not_a_setting() {
    let default = (DEFAULT_CAP_SECS, DEFAULT_GRACE_SECS);
    for bad in ["0", "-1", "-86400", "", "  ", "abc", "1.5", "0x10"] {
        let (g, _) = timed();
        g.store_for_tests()
            .policy_set("lease:cap", Some(bad), 0)
            .unwrap();
        g.store_for_tests()
            .policy_set("lease:grace", Some(bad), 0)
            .unwrap();
        let p = g.lease_policy().unwrap();
        assert_eq!((p.cap_secs, p.grace_secs), default, "{bad:?}");
    }
}

#[test]
fn one_second_and_a_padded_number_are_settings() {
    for (text, want) in [("1", 1), (" 7 ", 7), ("86400", 86_400)] {
        let (g, _) = timed();
        g.store_for_tests()
            .policy_set("lease:cap", Some(text), 0)
            .unwrap();
        g.store_for_tests()
            .policy_set("lease:grace", Some(text), 0)
            .unwrap();
        let p = g.lease_policy().unwrap();
        assert_eq!((p.cap_secs, p.grace_secs), (want, want), "{text:?}");
    }
}

/// A probe that answers one verdict and writes down the clock it was handed.
#[derive(Clone)]
struct Recorder {
    seen: Rc<RefCell<Vec<(String, u64)>>>,
    quiet_ms: Option<u64>,
}

impl LeaseSource for Recorder {
    fn observe(&self, member: &str, _session: &str, now_ms: u64) -> Option<Observed> {
        self.seen.borrow_mut().push((member.to_string(), now_ms));
        Some(Observed {
            verdict: Verdict::WaitingOnTool,
            quiet_for_ms: self.quiet_ms,
        })
    }
}

fn probed(quiet_ms: Option<u64>) -> (Gateway<SpyStore>, Rc<std::cell::Cell<i64>>, Recorder) {
    let (g, t) = timed();
    let rec = Recorder {
        seen: Rc::default(),
        quiet_ms,
    };
    let g = g.with_leases(rec.clone());
    g.store_for_tests()
        .note_session("sess-a", "alice", 0)
        .unwrap();
    g.claim(&alice(), "r", &ClaimSpec::new("p", 100, ""))
        .unwrap();
    (g, t, rec)
}

#[test]
fn the_probe_is_given_the_gateway_clock_in_milliseconds() {
    let (g, t, rec) = probed(None);
    for secs in [1_234, 1_000_000] {
        t.set(secs);
        rec.seen.borrow_mut().clear();
        g.claims(&bob(), "r").unwrap();
        assert_eq!(
            *rec.seen.borrow(),
            vec![("alice".to_string(), secs as u64 * 1000)]
        );
    }
}

#[test]
fn a_clock_before_the_epoch_is_given_to_the_probe_as_zero() {
    let (g, t, rec) = probed(None);
    t.set(-5);
    g.claims(&bob(), "r").unwrap();
    assert_eq!(*rec.seen.borrow(), vec![("alice".to_string(), 0)]);
}

#[test]
fn a_busy_holder_lapses_exactly_at_four_hours_without_progress_by_default() {
    for (quiet_s, lapses) in [(4 * 3600 - 1, false), (4 * 3600, true)] {
        let (g, t, _) = probed(Some(quiet_s * 1000));
        t.set(1_500);
        let taken = g.claim(&bob(), "r", &ClaimSpec::new("p", 100, "")).is_ok();
        assert_eq!(taken, lapses, "quiet for {quiet_s} s");
    }
}

#[test]
fn a_holder_that_is_working_is_kept_ten_minutes_ahead_of_now() {
    let (g, t, _) = probed(None);
    t.set(5_000);
    g.claims(&bob(), "r").unwrap();
    let held = g.claims(&bob(), "r").unwrap();
    assert_eq!(held[0].expires, 5_000 + DEFAULT_EXTEND_SECS);
}
