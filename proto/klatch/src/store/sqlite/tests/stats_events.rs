//! The event sections of the usage report (#1386, #1401, #1403, #1407, #1408): claim history, machine checkpoints,
//! machine status and ask routing, each counted from the rows the real paths wrote and narrowed by room and moment.

use super::stats_fixture::{D2, assert_counts, bench, bench_with_claims, exec, get, opts};
use crate::domain::StatsOptions;

/// The report of the claims bench narrowed to `room` (or all) and `since` (or the start).
fn report(room: Option<&str>, since: Option<i64>) -> crate::domain::UsageStats {
    bench_with_claims()
        .usage_stats(&StatsOptions {
            room: room.map(Into::into),
            since,
            ..opts()
        })
        .unwrap()
}

#[test]
fn ask_routing_outcomes_are_counted_per_ask_and_follow_room_and_since() {
    let r = report(None, None);
    let a = &r.ask_routing;
    assert_eq!(
        (a.routed, a.unrouted, a.taken, a.lapsed, a.escalated),
        (1, 1, 1, 1, 2)
    );
    assert_eq!(a.escalated_unreached, 1);
    let text = r.to_text();
    assert!(
        text.contains("ask routing: 1 routed to capable members, 1 routed to nobody, 1 taken, 1 lapsed, 2 escalated (1 with no coordinator to reach)"),
        "{text}"
    );
    assert_eq!(r.to_json()["ask_routing"]["escalated"], 2);
    let other = report(Some("vault"), None).ask_routing;
    assert_eq!(other.routed + other.escalated, 0);
    let late = report(None, Some(D2 + 10_000 + 850)).ask_routing;
    assert_eq!((late.escalated, late.routed), (1, 0));
}

#[test]
fn machine_status_updates_are_counted_by_class_and_follow_room_and_since() {
    let r = report(None, None);
    let m = &r.machine_status;
    assert_eq!((m.updates, m.working, m.idle, m.parked), (5, 3, 1, 1));
    let text = r.to_text();
    assert!(
        text.contains("machine status: 5 updates (working 3, idle 1, parked 1)"),
        "{text}"
    );
    assert!(!text.contains("(auto)") && !r.to_json().to_string().contains("TOPSECRET"));
    assert_eq!(r.to_json()["machine_status"]["working"], 3);
    // the passive claims line (#1401) and this one sit side by side in the text and in the JSON
    assert!(text.contains("passive claims: "), "{text}");
    assert!(r.to_json()["passive_claims"]["granted"].is_number());
    assert_eq!(report(Some("vault"), None).machine_status.updates, 0);
}

#[test]
fn machine_checkpoints_are_counted_with_their_bytes_and_follow_room_and_since() {
    let r = report(None, None);
    let m = &r.machine_checkpoints;
    let size = "turn ended; claims TOPSECRET/plan.rs; files TOPSECRET/plan.rs".len() as i64;
    assert_eq!((m.written, m.last, m.replaced), (3, 1, 1));
    assert_eq!(m.bytes, 3 * size);
    let text = r.to_text();
    assert!(
        text.contains(
            "machine checkpoints: 3 written (1 at session end), 1 replaced an earlier one"
        ),
        "{text}"
    );
    assert_eq!(r.to_json()["machine_checkpoints"]["written"], 3);
    assert_eq!(report(Some("vault"), None).machine_checkpoints.written, 0);
    let late = report(None, Some(D2 + 10_000 + 725));
    assert_eq!(late.machine_checkpoints.written, 1);
}

#[test]
fn claim_history_is_counted_by_class_and_word() {
    let h = report(None, None).claims.history;
    assert_counts(&h.granted, &[("file", 2), ("ticket", 2), ("other", 1)]);
    assert_counts(&h.refused, &[("same-path", 2)]);
    assert_counts(&h.released, &[("file", 2), ("other", 1)]);
    assert_counts(
        &h.released_by,
        &[("holder", 1), ("anchor", 1), ("session", 1)],
    );
    // cy's and dee's ticket leases lapse, and so does the bench's old file claim (held time unknown, so 0)
    assert_counts(&h.expired, &[("ticket", 2), ("file", 1)]);
    assert_eq!(h.takeovers, 1);
    assert_eq!((h.handled, h.handled_wait_median), (1, Some(90)));
    // holds: bob 70 s, the two lapsed tickets 100 s each, the merge claim 60 s, amy's 600 s (the old file claim has no
    // known start and is left out): median of 60, 70, 100, 100, 600
    assert_eq!((h.holds, h.hold_median), (5, Some(100)));
}

#[test]
fn claim_history_follows_room_and_since() {
    let none = report(Some("lobby"), None).claims.history;
    assert_eq!((none.granted.len(), none.handled, none.holds), (0, 0, 0));
    let late = report(None, Some(D2 + 10_450)).claims.history;
    assert_eq!(get(&late.granted, "other"), 1);
    assert_eq!(late.handled, 0);
}

#[test]
fn a_store_with_no_claim_record_reports_an_empty_history() {
    let s = bench();
    exec(&s, "DROP TABLE claim_events;");
    let h = s.usage_stats(&opts()).unwrap().claims.history;
    assert_eq!(h, crate::domain::ClaimHistory::default());
    let text = s.usage_stats(&opts()).unwrap().to_text();
    assert!(
        text.contains("claim hold time: median - over 0 holds"),
        "{text}"
    );
}
