//! Anchored claims (#125): a claim may name the factory's work item it is for; when the factory reports the item done
//! (push) or an adapter says so (poll), the claim is released and the path is free.

use super::*;
use crate::gateway::AnchorAdapter;
use std::cell::RefCell;
use std::collections::HashSet;

/// A fake factory: the set of work items it has finished, and the ones it cannot answer for.
#[derive(Default)]
struct Factory {
    done: HashSet<String>,
    broken: HashSet<String>,
    asked: RefCell<Vec<String>>,
}

impl AnchorAdapter for Factory {
    fn is_done(&self, anchor: &str) -> Result<bool> {
        self.asked.borrow_mut().push(anchor.to_string());
        if self.broken.contains(anchor) {
            return Err(Error::Backend("the factory did not answer".into()));
        }
        Ok(self.done.contains(anchor))
    }
}

fn anchored(path: &'static str, anchor: &'static str) -> ClaimSpec<'static> {
    ClaimSpec::new(path, 600, "").anchored(anchor)
}

#[test]
fn reporting_a_work_item_done_releases_its_claims_and_frees_the_paths() {
    let g = open();
    g.claim(&alice(), "r", &anchored("src/a.rs", "wo:7"))
        .unwrap();
    g.claim(&alice(), "q", &anchored("src/b.rs", "wo:7"))
        .unwrap();
    g.claim(&bob(), "r", &ClaimSpec::new("src/c.rs", 600, ""))
        .unwrap();
    assert!(
        g.claim(&bob(), "r", &ClaimSpec::new("src/a.rs", 600, ""))
            .is_err()
    );
    assert_eq!(g.anchor_done(&admin(), "wo:7").unwrap(), 2);
    g.claim(&bob(), "r", &ClaimSpec::new("src/a.rs", 600, ""))
        .unwrap();
    // the claim that was not anchored is untouched
    assert!(
        g.claims(&alice(), "r")
            .unwrap()
            .iter()
            .any(|c| c.path == "src/c.rs")
    );
    // the report is in the audit with its count
    let audit = g.admin_audit(&admin(), 5).unwrap();
    assert!(
        audit
            .iter()
            .any(|a| a.action == "anchor-done" && a.target == "wo:7" && a.detail.contains('2')),
        "{audit:?}"
    );
}

#[test]
fn a_report_for_an_item_nobody_claimed_for_changes_nothing() {
    let g = open();
    g.claim(&alice(), "r", &anchored("src/a.rs", "wo:7"))
        .unwrap();
    assert_eq!(g.anchor_done(&admin(), "wo:99").unwrap(), 0);
    assert_eq!(g.claims(&alice(), "r").unwrap().len(), 1);
}

#[test]
fn an_anchor_is_a_short_safe_name_and_a_bad_one_is_refused_before_anything_is_claimed() {
    let g = open();
    for bad in ["", "has space", "a\nb", "x;y", &"a".repeat(81)] {
        let spec = ClaimSpec::new("src/a.rs", 600, "").anchored(bad);
        assert!(is_invalid(g.claim(&alice(), "r", &spec)), "{bad:?}");
        assert!(is_invalid(g.anchor_done(&admin(), bad)), "{bad:?}");
    }
    assert!(g.claims(&alice(), "r").unwrap().is_empty());
    for ok in ["wo:7", "mission/42", "JIRA-123", "a.b_c"] {
        g.claim(&alice(), "r", &ClaimSpec::new(ok, 600, "").anchored(ok))
            .unwrap();
    }
}

#[test]
fn polling_an_adapter_releases_the_claims_of_finished_items_and_keeps_the_rest() {
    let g = open();
    g.claim(&alice(), "r", &anchored("src/a.rs", "wo:1"))
        .unwrap();
    g.claim(&alice(), "r", &anchored("src/b.rs", "wo:2"))
        .unwrap();
    g.claim(&bob(), "r", &anchored("src/c.rs", "wo:3")).unwrap();
    let mut f = Factory::default();
    f.done.insert("wo:1".into());
    f.done.insert("wo:3".into());
    assert_eq!(g.reap_anchors(&admin(), &f).unwrap(), 2);
    let held: Vec<String> = g
        .claims(&alice(), "r")
        .unwrap()
        .into_iter()
        .map(|c| c.path)
        .collect();
    assert_eq!(held, vec!["src/b.rs".to_string()]);
    assert_eq!(
        f.asked.borrow().len(),
        3,
        "each anchor in use is asked about once"
    );
}

#[test]
fn an_adapter_that_cannot_answer_leaves_the_claim_in_place_and_the_lease_still_ends_it() {
    let (g, t) = timed();
    g.claim(&alice(), "r", &anchored("src/a.rs", "wo:1"))
        .unwrap();
    let mut f = Factory::default();
    f.broken.insert("wo:1".into());
    assert_eq!(
        g.reap_anchors(&admin(), &f).unwrap(),
        0,
        "an unanswered item was released"
    );
    assert_eq!(g.claims(&alice(), "r").unwrap().len(), 1);
    t.set(t.get() + 601);
    assert!(
        g.claims(&alice(), "r").unwrap().is_empty(),
        "the lease no longer ends an anchored claim"
    );
}

#[test]
fn a_worker_cannot_report_an_item_done_by_typing_a_name_and_the_acts_go_through_the_guard() {
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(1),
        Rc::new(SpyGuard::denying(&["", "r"])),
    );
    assert!(is_refused(g.anchor_done(&alice(), "wo:7")));
    assert!(is_refused(g.reap_anchors(&alice(), &Factory::default())));
    assert_eq!(g.store.calls.get(), 0);
}

const PR_MERGED: &str = r#"{"action":"closed","pull_request":{"number":7,"state":"closed","merged":true},"repository":{"full_name":"Acme/Widgets"}}"#;
const PR_OPEN: &str = r#"{"action":"opened","pull_request":{"number":7,"state":"open"},"repository":{"full_name":"Acme/Widgets"}}"#;

#[test]
fn a_merged_pull_request_event_releases_the_claims_anchored_to_it_and_an_open_one_does_not() {
    let g = open();
    g.claim(&alice(), "r", &anchored("src/a.rs", "gh:pr:Acme/Widgets/7"))
        .unwrap();
    g.claim(&bob(), "q", &anchored("src/b.rs", "gh:pr:acme/widgets/8"))
        .unwrap();
    assert_eq!(g.github_events(&admin(), PR_OPEN).unwrap(), 0);
    assert_eq!(g.claims(&alice(), "r").unwrap().len(), 1);
    assert_eq!(g.github_events(&admin(), PR_MERGED).unwrap(), 1);
    assert!(g.claims(&alice(), "r").unwrap().is_empty());
    assert_eq!(
        g.claims(&bob(), "q").unwrap().len(),
        1,
        "another pull request was released"
    );
    assert!(
        g.admin_audit(&admin(), 10)
            .unwrap()
            .iter()
            .any(|a| a.action == "anchor-done" && a.target == "gh:pr:acme/widgets/7"),
        "the release is not audited"
    );
}

#[test]
fn a_closed_issue_event_releases_and_a_bad_payload_changes_nothing() {
    let g = open();
    g.claim(&alice(), "r", &anchored("src/a.rs", "gh:issue:a/b/9"))
        .unwrap();
    assert!(is_invalid(
        g.github_events(&admin(), "{\"state\":\"closed\"}")
    ));
    assert_eq!(g.claims(&alice(), "r").unwrap().len(), 1);
    let closed = r#"{"action":"closed","issue":{"number":9,"state":"closed"},"repository":{"full_name":"a/b"}}"#;
    assert_eq!(g.github_events(&admin(), closed).unwrap(), 1);
}

#[test]
fn github_events_go_through_the_guard_and_a_worker_is_refused_before_the_store() {
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(1),
        Rc::new(SpyGuard::denying(&["", "r"])),
    );
    assert!(is_refused(g.github_events(&alice(), PR_MERGED)));
    assert_eq!(g.store.calls.get(), 0);
}

fn session_claim(g: &Gateway<SpyStore>) {
    g.join(&alice(), "r").unwrap();
    g.note_session(&alice(), "s1").unwrap();
    g.claim(&alice(), "r", &anchored("src/a.rs", "session:s1"))
        .unwrap();
}

#[test]
fn a_claim_anchored_to_a_session_is_released_once_the_session_is_marked_done() {
    let g = open();
    session_claim(&g);
    let f = Factory::default();
    assert_eq!(
        g.reap_anchors(&admin(), &f).unwrap(),
        0,
        "a live session lost its claim"
    );
    g.set_status(&alice(), "r", "done session ended").unwrap();
    assert_eq!(g.reap_anchors(&admin(), &f).unwrap(), 1);
    assert!(g.claims(&alice(), "r").unwrap().is_empty());
    assert!(
        f.asked.borrow().iter().all(|a| !a.starts_with("session:")),
        "a session is klatch's own state: no adapter is asked"
    );
}

#[test]
fn an_unknown_session_and_a_session_done_in_only_one_room_keep_their_claims() {
    let g = open();
    session_claim(&g);
    g.join(&alice(), "q").unwrap();
    g.set_status(&alice(), "r", "done session ended").unwrap();
    g.claim(&alice(), "r", &anchored("src/b.rs", "session:nobody"))
        .unwrap();
    assert_eq!(g.reap_anchors(&admin(), &Factory::default()).unwrap(), 0);
    assert_eq!(g.claims(&alice(), "r").unwrap().len(), 2);
}

#[test]
fn a_session_anchor_may_be_released_by_name_through_anchor_done_too() {
    let g = open();
    session_claim(&g);
    assert_eq!(g.anchor_done(&admin(), "session:s1").unwrap(), 1);
}
