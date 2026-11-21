//! A credential renews on use (#161, decided 2026-11-18): each successful request pushes the expiry out by the idle
//! timeout, a hard cap from issue is never passed, a revoke still stops it at the next request, and each renewal is
//! audited. The clock is a test clock; the idle timeout is the spec's `ttl`.

use super::*;

fn spec(member: &str, ttl: i64, max_life: Option<i64>) -> CredentialSpec<'_> {
    CredentialSpec {
        member,
        clearance: "internal",
        ttl,
        max_life,
        parent: Some("lead"),
    }
}

/// A verified principal for `w-1` with an idle timeout of 600 s, issued at the test clock's 1000.
fn worker(max_life: Option<i64>) -> (Gateway<SpyStore>, Rc<std::cell::Cell<i64>>, Principal) {
    let (g, t) = timed();
    let token = g
        .admin_issue_credential(&admin(), &spec("w-1", 600, max_life))
        .unwrap();
    let who = g.verify_credential("w-1", &token).unwrap();
    g.join(&who, "r").unwrap();
    (g, t, who)
}

fn expires(g: &Gateway<SpyStore>) -> i64 {
    g.store.credential("w-1").unwrap().unwrap().expires
}

fn renewals(g: &Gateway<SpyStore>) -> usize {
    g.admin_audit(&admin(), 500)
        .unwrap()
        .iter()
        .filter(|a| a.action == "renew-credential" && a.target == "w-1")
        .count()
}

#[test]
fn a_successful_request_pushes_the_expiry_out_by_the_idle_timeout() {
    let (g, t, who) = worker(None);
    assert_eq!(expires(&g), 1_600);
    t.set(1_500);
    g.post(&who, "r", "still here").unwrap();
    assert_eq!(expires(&g), 2_100);
    // alive well past the issue lifetime because it was used
    t.set(2_099);
    g.post(&who, "r", "again").unwrap();
    assert_eq!(expires(&g), 2_699);
}

#[test]
fn an_idle_credential_still_dies_at_the_idle_timeout() {
    let (g, t, who) = worker(None);
    t.set(1_600);
    assert_eq!(
        rule_of(g.post(&who, "r", "late").unwrap_err()),
        "credential"
    );
    // the refused request did not bring it back
    t.set(1_601);
    assert!(is_refused(g.post(&who, "r", "later")));
}

#[test]
fn renewal_never_passes_the_hard_cap_and_the_cap_second_is_the_first_refused() {
    let (g, t, who) = worker(Some(1_000));
    t.set(1_300);
    g.post(&who, "r", "under the cap").unwrap();
    // 1300 + 600 is 1900, under the cap of 1000 + 1000 = 2000 from issue
    assert_eq!(expires(&g), 1_900);
    t.set(1_600);
    g.post(&who, "r", "x").unwrap();
    assert_eq!(expires(&g), 2_000);
    t.set(1_999);
    g.post(&who, "r", "last second").unwrap();
    assert_eq!(expires(&g), 2_000);
    t.set(2_000);
    assert_eq!(
        rule_of(g.post(&who, "r", "capped").unwrap_err()),
        "credential"
    );
}

#[test]
fn the_default_cap_is_24_hours_from_issue() {
    let (g, t, who) = worker(None);
    let mut now = 1_000;
    while now + 500 < 1_000 + 86_400 {
        now += 500;
        t.set(now);
        g.post(&who, "r", "heartbeat").unwrap();
    }
    assert_eq!(expires(&g), 1_000 + 86_400);
    t.set(1_000 + 86_400);
    assert_eq!(
        rule_of(g.post(&who, "r", "past").unwrap_err()),
        "credential"
    );
}

#[test]
fn a_revoke_stops_a_renewing_credential_at_its_next_request() {
    let (g, t, who) = worker(None);
    t.set(1_500);
    g.post(&who, "r", "x").unwrap();
    g.admin_revoke_credential(&admin(), "w-1").unwrap();
    t.set(1_501);
    assert_eq!(rule_of(g.post(&who, "r", "y").unwrap_err()), "credential");
    // a revoked credential is not renewed by the refused request
    assert_eq!(expires(&g), 2_100);
}

#[test]
fn a_renewal_is_audited_without_the_token_and_a_burst_of_requests_is_one_row() {
    let (g, t, who) = worker(None);
    assert_eq!(renewals(&g), 0);
    t.set(1_300);
    g.post(&who, "r", "a").unwrap();
    g.post(&who, "r", "b").unwrap();
    g.read(&who, "r", true).unwrap();
    assert_eq!(renewals(&g), 1);
    t.set(1_400);
    g.post(&who, "r", "c").unwrap();
    assert_eq!(renewals(&g), 2);
    let audit = g.admin_audit(&admin(), 50).unwrap();
    let row = audit
        .iter()
        .find(|a| a.action == "renew-credential")
        .unwrap();
    assert!(row.detail.contains("expires"), "{row:?}");
}

#[test]
fn an_unattributed_name_or_a_principal_with_no_credential_renews_nothing() {
    let (g, t, _who) = worker(None);
    t.set(1_300);
    g.post(&Principal::unattributed("w-1"), "r", "typed")
        .unwrap();
    g.join(&Principal::verified("peer-1"), "r").unwrap();
    assert_eq!(expires(&g), 1_600);
    assert_eq!(renewals(&g), 0);
}

#[test]
fn a_cap_outside_one_second_to_thirty_days_is_refused() {
    let g = open();
    for bad in [0, -1, 31 * 24 * 3600] {
        assert!(
            is_invalid(g.admin_issue_credential(&admin(), &spec("w-1", 600, Some(bad)))),
            "cap {bad}"
        );
    }
    g.admin_issue_credential(&admin(), &spec("w-1", 600, Some(30 * 24 * 3600)))
        .unwrap();
}

#[test]
fn a_cap_below_the_idle_timeout_limits_the_first_expiry_too() {
    let (g, _t, _who) = worker(Some(300));
    assert_eq!(expires(&g), 1_300);
}

#[test]
fn rotation_is_a_revoke_and_a_reissue_and_the_new_credential_has_a_fresh_cap() {
    let (g, t, _who) = worker(Some(1_000));
    t.set(1_800);
    g.admin_revoke_credential(&admin(), "w-1").unwrap();
    let token = g
        .admin_issue_credential(&admin(), &spec("w-1", 600, Some(1_000)))
        .unwrap();
    let fresh = g.verify_credential("w-1", &token).unwrap();
    assert_eq!(expires(&g), 2_400);
    t.set(2_300);
    g.post(&fresh, "r", "x").unwrap();
    assert_eq!(expires(&g), 2_800);
}

#[test]
fn the_listing_shows_the_cap_and_the_last_renewal() {
    let (g, t, who) = worker(Some(5_000));
    let before = g.admin_credentials(&admin()).unwrap();
    assert_eq!((before[0].cap, before[0].last_renewed), (6_000, None));
    t.set(1_500);
    g.post(&who, "r", "one").unwrap();
    t.set(1_900);
    g.post(&who, "r", "two").unwrap();
    let after = g.admin_credentials(&admin()).unwrap();
    assert_eq!((after[0].cap, after[0].last_renewed), (6_000, Some(1_900)));
}

#[test]
fn a_renewal_older_than_ten_thousand_audit_entries_is_still_reported() {
    let (g, t, who) = worker(Some(5_000));
    t.set(1_500);
    g.post(&who, "r", "one").unwrap();
    for i in 0..10_050 {
        g.store
            .record_audit(
                &crate::store::NewAudit::new("noise", "other", "x"),
                1_600 + i,
            )
            .unwrap();
    }
    let listed = g.admin_credentials(&admin()).unwrap();
    assert_eq!(listed[0].last_renewed, Some(1_500));
}

// #1027: the hooks that prove a session is alive renew its credential, so idle expiry is rarer and the end of the
// session can still mark the member done. Nothing from outside the hooks does it, and the cap still holds.

#[test]
fn a_worker_whose_hooks_keep_firing_does_not_idle_out() {
    let (g, t, who) = worker(None);
    let mut now = 1_000;
    // far more than one idle timeout (600) passes with nothing but hooks
    while now < 1_000 + 5_000 {
        now += 400;
        t.set(now);
        g.sign_of_life(&who).unwrap();
        g.record_hook_run(&who, "PostToolUse", 0).unwrap();
    }
    assert!(expires(&g) > now);
    g.set_status(&who, "r", "working").unwrap();
}

#[test]
fn the_stop_hook_alone_renews_too() {
    let (g, t, who) = worker(None);
    t.set(1_500);
    g.record_hook_run(&who, "Stop", 0).unwrap();
    assert_eq!(expires(&g), 2_100);
}

#[test]
fn hook_renewal_never_passes_the_cap_and_is_audited_without_the_token() {
    let (g, t, who) = worker(Some(1_000));
    t.set(1_900);
    g.sign_of_life(&who).unwrap();
    assert_eq!(expires(&g), 2_000);
    assert_eq!(renewals(&g), 1);
    let row = g
        .admin_audit(&admin(), 50)
        .unwrap()
        .into_iter()
        .find(|a| a.action == "renew-credential")
        .unwrap();
    assert!(row.detail.contains("expires=2000"), "{row:?}");
    t.set(2_000);
    assert_eq!(rule_of(g.sign_of_life(&who).unwrap_err()), "credential");
    assert_eq!(expires(&g), 2_000);
}

#[test]
fn a_session_end_hook_renews_nothing_and_an_idle_gap_past_the_limit_still_ends_as_before() {
    let (g, t, who) = worker(None);
    t.set(1_500);
    g.record_hook_run(&who, "SessionEnd", 0).unwrap();
    assert_eq!(expires(&g), 1_600);
    assert_eq!(renewals(&g), 0);
    // after an idle gap longer than the limit the hooks are refused and the end sequence behaves as before
    t.set(2_200);
    assert_eq!(rule_of(g.sign_of_life(&who).unwrap_err()), "credential");
    assert!(is_refused(g.record_hook_run(&who, "Stop", 0)));
    assert!(is_refused(g.set_status(&who, "r", "done session ended")));
    g.record_hook_run(&who, "SessionEnd", 0).unwrap();
    g.end_credential(&who).unwrap();
}

#[test]
fn a_request_that_is_not_a_hook_run_on_the_own_record_does_not_renew() {
    let (g, t, who) = worker(None);
    t.set(1_500);
    g.note_session(&who, "sess-1").unwrap();
    g.note_member_mode(&who, "default").unwrap();
    assert_eq!(expires(&g), 1_600);
}
