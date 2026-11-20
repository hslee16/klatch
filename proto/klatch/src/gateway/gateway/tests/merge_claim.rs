//! #865 (epic #863): the `merge:main` claim. Only an integrator or the human may take it; it serializes merging, so a
//! second integrator is refused and told who holds it; it has a short lease, an active holder renews it, a park
//! stretches it only up to a bound, and a holder that goes silent (a dead event forwarder, a hung session) lapses.

use super::*;
use crate::domain::{MERGE_CLAIM_MAX_SECS, MERGE_PARK_MAX_SECS};

const MERGE: &str = "merge:main";

fn ivy() -> Principal {
    Principal::unattributed("ivy")
}
fn ian() -> Principal {
    Principal::unattributed("ian")
}

/// A lobby with two integrators, ivy and ian, and an ordinary member, bob; the clock starts at 1000.
fn world() -> (Gateway<SpyStore>, Rc<std::cell::Cell<i64>>) {
    let (g, t) = timed();
    for who in [ivy(), ian(), bob()] {
        g.join(&who, "lobby").unwrap();
    }
    g.admin_set_integrator(&admin(), "ivy", true).unwrap();
    g.admin_set_integrator(&admin(), "ian", true).unwrap();
    (g, t)
}

fn take(g: &Gateway<SpyStore>, who: &Principal, ttl: i64) -> Result<()> {
    g.claim(who, "lobby", &ClaimSpec::new(MERGE, ttl, "merging"))
}

/// When the claim lapses, if it is held.
fn expiry(g: &Gateway<SpyStore>, path: &str) -> Option<i64> {
    g.claims(&bob(), "lobby")
        .unwrap()
        .into_iter()
        .find(|c| c.path == path)
        .map(|c| c.expires)
}

#[test]
fn only_the_human_grants_the_role_and_it_is_one_role_at_a_time() {
    let (g, _) = world();
    assert_eq!(g.role_of("ivy").unwrap(), Some(Role::Integrator));
    // another role replaces it, and --off removes it
    g.admin_set_librarian(&admin(), "ivy", true).unwrap();
    assert_eq!(g.role_of("ivy").unwrap(), Some(Role::Librarian));
    g.admin_set_integrator(&admin(), "ivy", true).unwrap();
    g.admin_set_integrator(&admin(), "ivy", false).unwrap();
    assert_eq!(g.role_of("ivy").unwrap(), None);
    let log = g.admin_audit(&admin(), 20).unwrap();
    assert!(log.iter().any(|a| a.action == "integrator"));
    // on the members' gateway neither an integrator nor the admin agent can give or take it, and no typed name
    // reaches the human's path
    g.admin_set_admin_agent(&admin(), "ada", true).unwrap();
    g.admin_set_integrator(&admin(), "ivy", true).unwrap();
    let m = g.for_members();
    assert!(is_refused(m.admin_set_integrator(&ivy(), "bob", true)));
    assert!(is_refused(m.admin_set_integrator(
        &Principal::unattributed("ada"),
        "bob",
        true
    )));
    assert!(
        is_refused(take(&m, &admin(), 600)),
        "the name admin is not a member's"
    );
    assert_eq!(m.role_of("bob").unwrap(), None);
}

#[test]
fn an_integrator_takes_the_claim() {
    let (g, _) = world();
    take(&g, &ivy(), 600).unwrap();
    assert_eq!(expiry(&g, MERGE), Some(1_600));
}

#[test]
fn a_plain_member_is_refused_with_the_rule_and_the_next_step() {
    let (g, _) = world();
    let e = take(&g, &bob(), 600).unwrap_err();
    let Error::Refused { rule, reason } = e else {
        panic!("not a refusal: {e}");
    };
    assert_eq!(rule, "integrator");
    assert!(reason.contains("klatch admin integrator"), "{reason}");
    assert_eq!(expiry(&g, MERGE), None);
    // the lease question the command line asks first is refused the same way
    let e = g
        .claim_ttl_for(&bob(), ("lobby", MERGE), Some(600))
        .unwrap_err();
    assert_eq!(rule_of(e), "integrator");
}

#[test]
fn the_name_cannot_be_spelled_around_the_rule() {
    let (g, _) = world();
    for spelled in ["Merge:Main", " merge:main ", "MERGE:MAIN"] {
        let e = g
            .claim(&bob(), "lobby", &ClaimSpec::new(spelled, 600, ""))
            .unwrap_err();
        assert_eq!(rule_of(e), "integrator", "{spelled:?}");
    }
    // an integrator taking it in another spelling holds the one claim
    g.claim(&ivy(), "lobby", &ClaimSpec::new("Merge:Main", 600, ""))
        .unwrap();
    assert_eq!(expiry(&g, MERGE), Some(1_600));
    assert!(take(&g, &ian(), 600).is_err());
}

#[test]
fn the_admin_agent_and_the_other_roles_are_refused() {
    let (g, _) = world();
    g.admin_set_admin_agent(&admin(), "ada", true).unwrap();
    g.admin_set_facilitator(&admin(), "fay", true).unwrap();
    for who in ["ada", "fay"] {
        g.join(&Principal::unattributed(who), "lobby").ok();
        let e = take(&g, &Principal::unattributed(who), 600).unwrap_err();
        assert!(matches!(e, Error::Refused { .. }), "{who}");
    }
    let e = take(&g, &Principal::unattributed("ada"), 600).unwrap_err();
    assert_eq!(rule_of(e), "integrator");
    assert_eq!(expiry(&g, MERGE), None);
}

#[test]
fn the_human_terminal_path_may_take_it_and_a_removed_integrator_may_not() {
    let (g, _) = world();
    take(&g, &admin(), 600).unwrap();
    g.release(&admin(), "lobby", MERGE).unwrap();
    g.admin_set_integrator(&admin(), "ivy", false).unwrap();
    assert_eq!(rule_of(take(&g, &ivy(), 600).unwrap_err()), "integrator");
}

#[test]
fn a_second_integrator_is_refused_and_told_who_holds_it() {
    let (g, _) = world();
    take(&g, &ivy(), 600).unwrap();
    let e = take(&g, &ian(), 600).unwrap_err();
    assert!(!matches!(e, Error::Backend(_)));
    assert!(e.to_string().contains("ivy"), "names the holder: {e}");
    // the holder renews its own claim by claiming again
    take(&g, &ivy(), 600).unwrap();
}

#[test]
fn it_is_one_claim_across_scopes() {
    let (g, _) = world();
    g.set_scope(&ivy(), "git:one").unwrap();
    g.set_scope(&ian(), "git:two").unwrap();
    take(&g, &ivy(), 600).unwrap();
    assert!(
        take(&g, &ian(), 600).is_err(),
        "a different repository scope does not make a second merge:main"
    );
}

#[test]
fn it_lapses_after_its_lease_and_is_takeable_again() {
    let (g, t) = world();
    take(&g, &ivy(), 600).unwrap();
    t.set(1_599);
    assert!(
        take(&g, &ian(), 600).is_err(),
        "still held inside the lease"
    );
    t.set(1_601);
    assert_eq!(expiry(&g, MERGE), None, "lapsed");
    take(&g, &ian(), 600).unwrap();
    assert_eq!(expiry(&g, MERGE), Some(2_201));
}

#[test]
fn the_lease_is_capped_and_an_active_holder_renews_it() {
    let (g, t) = world();
    take(&g, &ivy(), 86_400).unwrap();
    assert_eq!(expiry(&g, MERGE), Some(1_000 + MERGE_CLAIM_MAX_SECS));
    // a member that is active keeps its claim for the lease it was made with, no longer
    t.set(2_000);
    g.post(&ivy(), "lobby", "merging #12").unwrap();
    assert_eq!(expiry(&g, MERGE), Some(2_000 + MERGE_CLAIM_MAX_SECS));
}

#[test]
fn a_park_extends_it_only_up_to_the_bound() {
    let (g, t) = world();
    take(&g, &ivy(), 600).unwrap();
    g.claim(&ivy(), "lobby", &ClaimSpec::new("src/lib.rs", 600, ""))
        .unwrap();
    t.set(1_100);
    g.post(&ivy(), "lobby", "parked: waiting on checks, back by 3h")
        .unwrap();
    // any other claim is held to the return time; merge:main only to the bound
    assert_eq!(expiry(&g, "src/lib.rs"), Some(1_100 + 3 * 3600));
    assert_eq!(expiry(&g, MERGE), Some(1_100 + MERGE_PARK_MAX_SECS));
    assert!(MERGE_PARK_MAX_SECS < 3600, "never an hour (#533)");
    // a shorter park is honored as it is
    g.post(&ivy(), "lobby", "parked: one check, back by 5m")
        .unwrap();
    assert!(expiry(&g, MERGE).unwrap() <= 1_100 + MERGE_PARK_MAX_SECS);
}

#[test]
fn a_holder_that_parked_for_an_event_and_went_silent_lapses_at_the_bound() {
    // the integrator waits for a check suite event while it holds the claim; if the forwarder dies no event wakes
    // it, it never posts, and the claim must not stay held (#863)
    let (g, t) = world();
    take(&g, &ivy(), 600).unwrap();
    t.set(1_100);
    g.post(
        &ivy(),
        "lobby",
        "parked: waiting on a check suite, back by 2h",
    )
    .unwrap();
    let bound = 1_100 + MERGE_PARK_MAX_SECS;
    t.set(bound - 1);
    assert!(take(&g, &ian(), 600).is_err(), "held while the park holds");
    t.set(bound + 1);
    assert_eq!(expiry(&g, MERGE), None);
    take(&g, &ian(), 600).unwrap();
    assert_eq!(g.claims(&bob(), "lobby").unwrap()[0].owner, "ian");
}

#[test]
fn a_holder_that_wakes_from_its_park_gets_a_normal_lease_back() {
    let (g, t) = world();
    take(&g, &ivy(), 600).unwrap();
    t.set(1_100);
    g.post(&ivy(), "lobby", "parked: waiting on checks, back by 3h")
        .unwrap();
    t.set(1_200);
    g.post(&ivy(), "lobby", "the checks passed, merging")
        .unwrap();
    assert_eq!(expiry(&g, MERGE), Some(1_200 + 600));
}
