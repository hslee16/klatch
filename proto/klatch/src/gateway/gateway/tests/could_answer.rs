//! `could_answer` (#671, part 1): the read-only check "could member X answer ask N, and if not, why". It reads policy
//! and the store and changes nothing. It is the check a supervisor outside klatch asks before it starts a helper.

use super::*;
use crate::domain::CouldAnswer;

fn helper() -> Principal {
    Principal::unattributed("helper")
}

/// A room "r" with an ask from alice; returns the gateway and the ask number.
fn with_ask() -> (Gateway<SpyStore>, i64) {
    let g = open();
    g.join(&alice(), "r").unwrap();
    let seq = g.post(&alice(), "r", "seeking: a thing").unwrap().seq;
    (g, seq)
}

fn rules(c: &CouldAnswer) -> Vec<&str> {
    c.blockers.iter().map(|b| b.rule.as_str()).collect()
}

fn check(g: &Gateway<SpyStore>, member: &str, seq: i64) -> CouldAnswer {
    g.could_answer(&admin(), "r", member, seq).unwrap()
}

#[test]
fn a_cleared_idle_member_could_answer_an_open_ask() {
    let (g, seq) = with_ask();
    let c = check(&g, "helper", seq);
    assert!(c.could(), "{c:?}");
    assert_eq!((c.member.as_str(), c.ask), ("helper", seq));
}

#[test]
fn an_ask_that_is_not_open_is_named_and_nothing_else_is_judged() {
    let (g, seq) = with_ask();
    assert_eq!(rules(&check(&g, "helper", seq + 50)), ["no-such-ask"]);
    g.post(&bob(), "r", &format!("answer to #{seq}: done"))
        .unwrap();
    assert_eq!(rules(&check(&g, "helper", seq)), ["no-such-ask"]);
}

#[test]
fn a_member_whose_clearance_is_below_the_room_label_cannot() {
    let (g, seq) = with_ask();
    g.admin_set_room_label(&admin(), "r", "internal").unwrap();
    let c = check(&g, "helper", seq);
    assert_eq!(rules(&c), ["clearance"]);
    assert!(c.blockers[0].reason.contains("helper"), "{c:?}");
    g.admin_set_clearance(&admin(), "helper", "internal")
        .unwrap();
    assert!(check(&g, "helper", seq).could());
}

#[test]
fn a_room_grant_counts_as_clearance_in_that_room() {
    let (g, seq) = with_ask();
    g.admin_set_room_label(&admin(), "r", "internal").unwrap();
    g.admin_set_clearance(&admin(), "alice", "internal")
        .unwrap();
    assert!(!check(&g, "helper", seq).could());
    g.grant_room_clearance(&alice(), "r", "helper", "internal")
        .unwrap();
    assert!(check(&g, "helper", seq).could());
}

#[test]
fn roles_that_may_not_post_an_answer_cannot() {
    let (g, seq) = with_ask();
    g.admin_set_librarian(&admin(), "helper", true).unwrap();
    assert_eq!(rules(&check(&g, "helper", seq)), ["read-only"]);
    g.admin_set_reviewer(&admin(), "helper", true).unwrap();
    assert_eq!(rules(&check(&g, "helper", seq)), ["reviewer"]);
    g.admin_set_facilitator(&admin(), "helper", true).unwrap();
    assert_eq!(rules(&check(&g, "helper", seq)), ["facilitator"]);
    g.admin_set_integrator(&admin(), "helper", true).unwrap();
    assert!(check(&g, "helper", seq).could());
}

#[test]
fn a_removed_member_and_a_closed_room_and_a_pause_each_refuse() {
    let (g, seq) = with_ask();
    g.admin_remove_member(&admin(), "r", "helper").unwrap();
    assert_eq!(rules(&check(&g, "helper", seq)), ["admin-block"]);
    g.admin_allow_member(&admin(), "r", "helper").unwrap();
    g.admin_close_room(&admin(), "r", true).unwrap();
    assert_eq!(rules(&check(&g, "helper", seq)), ["admin-close"]);
    g.admin_close_room(&admin(), "r", false).unwrap();
    g.admin_pause(&admin(), true).unwrap();
    assert_eq!(rules(&check(&g, "helper", seq)), ["admin-pause"]);
}

#[test]
fn the_asker_cannot_answer_its_own_ask() {
    let (g, seq) = with_ask();
    assert_eq!(rules(&check(&g, "alice", seq)), ["own-ask"]);
}

#[test]
fn a_claimed_ask_and_a_busy_member_are_named() {
    let (g, seq) = with_ask();
    g.claim(&bob(), "r", &ClaimSpec::new(&format!("ask:{seq}"), 500, ""))
        .unwrap();
    assert_eq!(rules(&check(&g, "helper", seq)), ["ask-claimed"]);
    // a second ask nobody holds; the helper is busy on a claim elsewhere
    let two = g.post(&alice(), "r", "seeking: another").unwrap().seq;
    assert!(check(&g, "helper", two).could());
    g.join(&helper(), "r").unwrap();
    g.claim(&helper(), "r", &ClaimSpec::new("src/x.rs", 500, ""))
        .unwrap();
    assert_eq!(rules(&check(&g, "helper", two)), ["busy"]);
}

#[test]
fn a_member_whose_credential_was_revoked_cannot() {
    let (g, seq) = with_ask();
    let spec = CredentialSpec {
        member: "helper",
        clearance: "public",
        ttl: 600,
        max_life: None,
        parent: None,
    };
    g.admin_issue_credential(&admin(), &spec).unwrap();
    assert!(check(&g, "helper", seq).could());
    g.admin_revoke_credential(&admin(), "helper").unwrap();
    assert_eq!(rules(&check(&g, "helper", seq)), ["credential"]);
}

#[test]
fn several_reasons_are_all_listed() {
    let (g, seq) = with_ask();
    g.admin_set_room_label(&admin(), "r", "internal").unwrap();
    g.admin_set_librarian(&admin(), "helper", true).unwrap();
    assert_eq!(rules(&check(&g, "helper", seq)), ["clearance", "read-only"]);
}

#[test]
fn the_check_changes_nothing() {
    let (g, seq) = with_ask();
    let count = |g: &Gateway<SpyStore>| {
        (
            g.tail(&alice(), "r", (0, 50)).unwrap().len(),
            g.admin_audit(&admin(), 50).unwrap().len(),
            g.claims(&alice(), "r").unwrap().len(),
        )
    };
    let before = count(&g);
    let _ = check(&g, "helper", seq);
    let _ = check(&g, "helper", seq + 9);
    assert_eq!(before, count(&g));
}

#[test]
fn a_member_may_check_itself_and_only_an_admin_may_check_another() {
    let (g, seq) = with_ask();
    g.admin_set_admin_agent(&admin(), "ada", true).unwrap();
    let g = g.for_members();
    assert!(
        g.could_answer(&helper(), "r", "helper", seq)
            .unwrap()
            .could()
    );
    let e = g.could_answer(&helper(), "r", "bob", seq).unwrap_err();
    assert_eq!(rule_of(e), "admin-scope");
    let ada = Principal::unattributed("ada");
    let e = g.could_answer(&ada, "r", "bob", seq).unwrap_err();
    assert_eq!(rule_of(e), "admin-scope");
}

#[test]
fn checking_oneself_in_a_room_above_ones_clearance_is_refused() {
    let (g, seq) = with_ask();
    g.admin_set_room_label(&admin(), "r", "internal").unwrap();
    let e = g.could_answer(&helper(), "r", "helper", seq).unwrap_err();
    assert_eq!(rule_of(e), "clearance");
}

#[test]
fn a_bad_member_name_is_invalid() {
    let (g, seq) = with_ask();
    assert!(is_invalid(g.could_answer(&admin(), "r", "bad name", seq)));
}

// An ask may declare a scope (#1171); the check blocks only when both the ask and the member declare one and they
// differ.

/// A room "r" with an ask from alice that declares `scope` (or none), and the helper's own scope.
fn with_scopes(ask_scope: Option<&str>, member_scope: Option<&str>) -> (Gateway<SpyStore>, i64) {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.join(&helper(), "r").unwrap();
    let seq = match ask_scope {
        Some(s) => {
            g.post_scoped(&alice(), ("r", "seeking: a thing", None), s)
                .unwrap()
                .seq
        }
        None => g.post(&alice(), "r", "seeking: a thing").unwrap().seq,
    };
    if let Some(s) = member_scope {
        g.set_scope(&helper(), s).unwrap();
    }
    (g, seq)
}

#[test]
fn differing_declared_scopes_block_and_the_reason_names_both() {
    let (g, seq) = with_scopes(Some("git:foo"), Some("git:bar"));
    let c = check(&g, "helper", seq);
    assert_eq!(rules(&c), ["scope"], "{c:?}");
    let why = &c.blockers[0].reason;
    assert!(
        why.contains("git:foo") && why.contains("git:bar") && why.contains("helper"),
        "{why}"
    );
}

#[test]
fn equal_declared_scopes_do_not_block() {
    let (g, seq) = with_scopes(Some("git:foo"), Some("git:foo"));
    assert!(check(&g, "helper", seq).could());
}

#[test]
fn an_ask_with_no_scope_or_a_member_with_none_changes_nothing() {
    let (g, seq) = with_scopes(None, Some("git:bar"));
    assert!(check(&g, "helper", seq).could(), "ask without a scope");
    let (g, seq) = with_scopes(Some("git:foo"), None);
    assert!(check(&g, "helper", seq).could(), "member without a scope");
    let (g, seq) = with_scopes(None, None);
    assert!(check(&g, "helper", seq).could(), "neither");
}

#[test]
fn a_scope_on_the_ask_is_stored_and_shown_by_open_asks() {
    let (g, seq) = with_scopes(Some("git:foo"), None);
    let none = g.post(&alice(), "r", "seeking: plain").unwrap().seq;
    let asks = g.open_asks(&alice(), "r").unwrap();
    let scope = |n: i64| asks.iter().find(|a| a.seq == n).unwrap().scope.clone();
    assert_eq!(scope(seq).as_deref(), Some("git:foo"));
    assert_eq!(scope(none), None);
}

#[test]
fn a_bad_scope_or_a_scope_on_something_that_is_not_an_ask_is_refused_and_posts_nothing() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    let bad = g.post_scoped(&alice(), ("r", "seeking: x", None), "has space");
    assert!(
        matches!(bad, Err(crate::domain::Error::Invalid(_))),
        "{bad:?}"
    );
    let not_ask = g.post_scoped(&alice(), ("r", "just talking", None), "git:foo");
    assert!(
        matches!(not_ask, Err(crate::domain::Error::Invalid(_))),
        "{not_ask:?}"
    );
    assert!(g.read(&bob(), "r", false).unwrap().is_empty());
}

#[test]
fn a_reader_sees_the_scope_on_the_ask_message_and_not_on_others() {
    let (g, seq) = with_scopes(Some("git:foo"), None);
    g.post(&alice(), "r", "chatter").unwrap();
    let seen = g.read(&bob(), "r", false).unwrap();
    let scope = |n: i64| seen.iter().find(|m| m.seq == n).unwrap().scope.clone();
    assert_eq!(scope(seq).as_deref(), Some("git:foo"));
    assert_eq!(scope(seq + 1), None);
}
