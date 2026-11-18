//! Zero trust in code (#161, R35): no operation reaches the store without the guard. Every public method of the
//! gateway is listed below, as guarded or as acting only on the caller's own record, and a test fails when a new
//! method is not listed, so a new operation cannot skip the decision.

use super::*;
use crate::gateway::ClaimSpec;

/// Every public method that takes a room or acts on anyone but the caller: it asks the guard first, and under a
/// guard that refuses everything it is refused with nothing read or written.
type Call = fn(&Gateway<SpyStore>, &Principal) -> bool;

/// An adapter that knows nothing: only used to prove the call is refused before it is asked.
struct NoFactory;
impl crate::gateway::AnchorAdapter for NoFactory {
    fn is_done(&self, _anchor: &str) -> Result<bool> {
        Ok(false)
    }
}

fn refused<T>(r: Result<T>) -> bool {
    matches!(r, Err(Error::Refused { .. }))
}

const GUARDED: &[(&str, Call)] = &[
    ("join", |g, w| refused(g.join(w, "r"))),
    ("post", |g, w| refused(g.post(w, "r", "x"))),
    ("post_labeled", |g, w| {
        refused(g.post_labeled(w, ("r", "x", None)))
    }),
    ("post_handoff", |g, w| {
        refused(g.post_handoff(
            w,
            ("r", "x", None),
            &crate::gateway::Handoff {
                kind: "k",
                payload: None,
            },
        ))
    }),
    ("dm", |g, w| refused(g.dm(w, "bob", "x", None))),
    ("read", |g, w| refused(g.read(w, "r", true))),
    ("tail", |g, w| refused(g.tail(w, "r", (0, 10)))),
    ("history", |g, w| {
        refused(g.history(w, "r", crate::gateway::HistoryRange::default()))
    }),
    ("head", |g, w| refused(g.head(w, "r"))),
    ("ack", |g, w| refused(g.ack(w, "r", 1))),
    ("who", |g, w| refused(g.who(w, "r"))),
    ("receipt", |g, w| refused(g.receipt(w, "r", 1))),
    ("board", |g, w| refused(g.board(w, "r"))),
    ("open_asks", |g, w| refused(g.open_asks(w, "r"))),
    ("claim", |g, w| {
        refused(g.claim(w, "r", &ClaimSpec::new("p", 10, "")))
    }),
    ("invite", |g, w| refused(g.invite(w, "bob", "r", "r"))),
    ("process_exited", |g, w| {
        refused(g.process_exited(w, "r", "bob"))
    }),
    ("request_crossing", |g, w| {
        refused(g.request_crossing(w, "r", 1, "r"))
    }),
    // before admin_close_room below: a closed room refuses before the guard is asked
    ("member_card", |g, w| refused(g.member_card(w, "r", "bob"))),
    ("claim_ttl_for", |g, w| {
        refused(g.claim_ttl_for(w, ("r", "ask:1"), None))
    }),
    ("claims", |g, w| refused(g.claims(w, "r"))),
    ("cycle_notices", |g, w| refused(g.cycle_notices(w, "r"))),
    ("release", |g, w| refused(g.release(w, "r", "p"))),
    ("release_with_note", |g, w| {
        refused(g.release_with_note(w, ("r", "p"), "n"))
    }),
    ("last_release", |g, w| refused(g.last_release(w, "r", "p"))),
    ("set_status", |g, w| refused(g.set_status(w, "r", "done"))),
    ("set_muted", |g, w| refused(g.set_muted(w, "r", true))),
    ("wait_poll", |g, w| refused(g.wait_poll(w, "r"))),
    ("wait_seen", |g, w| refused(g.wait_seen(w, "r", 1))),
    ("wake_mark", |g, w| refused(g.wake_mark(w, "r"))),
    ("wakeable", |g, w| refused(g.wakeable(w, "r"))),
    ("wake_summary", |g, w| refused(g.wake_summary(w, "r"))),
    // another member's data, and housekeeping that deletes other members
    ("hook_runs", |g, w| refused(g.hook_runs(w, "bob"))),
    ("prune_members", |g, w| refused(g.prune_members(w, 0))),
    // the administrative acts
    ("admin_close_room", |g, w| {
        refused(g.admin_close_room(w, "r", true))
    }),
    ("admin_pause", |g, w| refused(g.admin_pause(w, true))),
    ("admin_remove_member", |g, w| {
        refused(g.admin_remove_member(w, "r", "bob"))
    }),
    ("admin_allow_member", |g, w| {
        refused(g.admin_allow_member(w, "r", "bob"))
    }),
    ("admin_audit", |g, w| refused(g.admin_audit(w, 5))),
    ("admin_issue_credential", |g, w| {
        refused(g.admin_issue_credential(
            w,
            &CredentialSpec {
                member: "w-1",
                clearance: "internal",
                ttl: 60,
                parent: None,
            },
        ))
    }),
    ("admin_revoke_credential", |g, w| {
        refused(g.admin_revoke_credential(w, "w-1"))
    }),
    ("admin_credentials", |g, w| refused(g.admin_credentials(w))),
    ("admin_run_create", |g, w| {
        refused(g.admin_run_create(w, "a"))
    }),
    ("admin_run_end", |g, w| {
        refused(g.admin_run_end(w, "a", None))
    }),
    ("admin_runs", |g, w| refused(g.admin_runs(w))),
    ("anchor_done", |g, w| refused(g.anchor_done(w, "wo:1"))),
    ("github_events", |g, w| {
        refused(g.github_events(
            w,
            r#"{"number":1,"state":"closed","repository":{"full_name":"a/b"}}"#,
        ))
    }),
    ("reap_anchors", |g, w| {
        refused(g.reap_anchors(w, &NoFactory))
    }),
    ("admin_seal_label", |g, w| {
        refused(g.admin_seal_label(w, "private"))
    }),
    ("admin_note", |g, w| {
        refused(g.admin_note(w, ("key-test", "x"), "y"))
    }),
    ("admin_set_room_label", |g, w| {
        refused(g.admin_set_room_label(w, "r", "private"))
    }),
    ("admin_set_clearance", |g, w| {
        refused(g.admin_set_clearance(w, "bob", "private"))
    }),
    ("admin_set_cap", |g, w| {
        refused(g.admin_set_cap(w, "private"))
    }),
    ("admin_set_label_chain", |g, w| {
        refused(g.admin_set_label_chain(w, &["a", "b"]))
    }),
    ("admin_set_room_stale", |g, w| {
        refused(g.admin_set_room_stale(w, "r", Some(120)))
    }),
    ("admin_set_room_ask_lease", |g, w| {
        refused(g.admin_set_room_ask_lease(w, "r", Some(120)))
    }),
    ("admin_plant_canary", |g, w| {
        refused(g.admin_plant_canary(w, "r"))
    }),
    ("admin_refusals", |g, w| refused(g.admin_refusals(w, 5))),
    ("admin_add_scrub_term", |g, w| {
        refused(g.admin_add_scrub_term(w, "n", "t"))
    }),
    ("admin_remove_scrub_term", |g, w| {
        refused(g.admin_remove_scrub_term(w, "n"))
    }),
    ("admin_scrub_terms", |g, w| refused(g.admin_scrub_terms(w))),
    // found by the directory scan (#161): these files were not in the hand-kept list before
    ("admin_set_librarian", |g, w| {
        refused(g.admin_set_librarian(w, "bob", true))
    }),
    ("admin_set_reviewer", |g, w| {
        refused(g.admin_set_reviewer(w, "bob", true))
    }),
    ("admin_set_admin_agent", |g, w| {
        refused(g.admin_set_admin_agent(w, "bob", true))
    }),
    ("admin_set_room_creation_granted", |g, w| {
        refused(g.admin_set_room_creation_granted(w, true))
    }),
    ("admin_grant_room_capability", |g, w| {
        refused(g.admin_grant_room_capability(w, "bob", true))
    }),
    ("admin_set_rate_limit", |g, w| {
        refused(g.admin_set_rate_limit(w, Some((5, 60))))
    }),
    ("admin_set_lease_cap", |g, w| {
        refused(g.admin_set_lease_cap(w, Some(60)))
    }),
    ("admin_set_lease_grace", |g, w| {
        refused(g.admin_set_lease_grace(w, Some(60)))
    }),
    ("review_queue", |g, w| refused(g.review_queue(w, 5))),
    ("review_next", |g, w| refused(g.review_next(w))),
    ("review_verdict", |g, w| {
        refused(g.review_verdict(w, 1, &crate::domain::Verdict::Allow))
    }),
];

/// Methods that are not an operation on shared state: building the gateway, the clock, or reading or writing only
/// the caller's own record (its scope, parent, session id, hook run record, rooms it joined, claims it holds), or a
/// lookup of policy that the gateway itself uses. Each has a reason; a new one needs one.
const OWN_RECORD: &[(&str, &str)] = &[
    ("new", "builds the gateway"),
    (
        "with_notifier",
        "builds the gateway: who is told of a stored post",
    ),
    (
        "for_members",
        "builds the gateway: members' admin acts need the admin agent role",
    ),
    ("now", "the clock"),
    (
        "change_token",
        "when to look, never what: a number that moves when any connection commits, with no room, member or text in it. It takes no principal and returns nothing a peer sees; a session or wait uses it only to decide when to run the checked poll (#616)",
    ),
    (
        "my_refusals",
        "the caller's own refusal record: it reads only rows of the caller's principal name, so it is the same as reading its own hook record, and a member stopped by the breaker must still read why (#678)",
    ),
    (
        "digest",
        "asks the guard once per room and reports the rooms it refused",
    ),
    ("digest_with", "the same, with the backlog"),
    ("note_parent", "the caller's own parent link"),
    ("note_session", "the caller's own session id"),
    (
        "note_member_mode",
        "the caller's own permission mode, a short word with no content",
    ),
    (
        "record_stop",
        "the caller's own park, from the end of its own turn",
    ),
    (
        "sign_of_life",
        "the caller's own park, ended by its own hook",
    ),
    ("scope_of", "the caller's own scope"),
    ("set_scope", "the caller's own scope"),
    ("set_kind", "the caller's own kind, display only"),
    ("kind_of", "the caller's own kind"),
    ("record_hook_run", "the caller's own hook run record"),
    ("release_all", "releases only the claims the caller holds"),
    ("rooms", "the rooms the caller joined"),
    (
        "sealed_labels",
        "a policy lookup the CLI uses to find which keys to load; takes no principal",
    ),
    (
        "sealed_labels_within",
        "the same, limited to what one member's clearance reaches; takes no principal",
    ),
    (
        "end_credential",
        "a worker revokes only its own credential, and only as a verified principal",
    ),
    (
        "verify_credential",
        "authentication itself: it takes proof of identity instead of a principal, and records a failure",
    ),
    (
        "use_label_keys",
        "the keys this process holds, in memory only",
    ),
    (
        "clearance_of",
        "a policy lookup the gateway uses on every call; takes no principal",
    ),
    (
        "room_label",
        "a policy lookup the gateway uses on every call; takes no principal",
    ),
    (
        "role_of",
        "a policy lookup the gateway uses on every call; takes no principal",
    ),
    ("is_admin_agent", "a role lookup; takes no principal"),
    ("is_librarian", "a role lookup; takes no principal"),
    (
        "can_create_rooms",
        "a policy lookup `check_room_capability` uses on every call; takes no principal",
    ),
    (
        "lease_policy",
        "a policy lookup the lease rules use; takes no principal",
    ),
    (
        "rate_limit",
        "a policy lookup the budget check uses on every post; takes no principal",
    ),
    (
        "with_leases",
        "builds the gateway: where liveness comes from",
    ),
    (
        "delivery_view",
        "the caller's own delivery view, a key built from the caller's own name",
    ),
    (
        "set_delivery_view",
        "the caller's own delivery view, a key built from the caller's own name",
    ),
    (
        "release_crossings",
        "applies only verdicts a reviewer or judge already recorded, with the floors checked again; it takes no principal and adds no authority. `klatch judge run --once` reaches it behind the launcher heuristic only, a gap kept in proto/AUTHZ-AUDIT.md",
    ),
];

/// The source files of the gateway, found by reading the directory so that a new file cannot be left out of the
/// scan (#161): `gateway.rs` and every file beside it, tests excluded.
fn gateway_sources() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/gateway");
    let mut files = vec![root.join("gateway.rs")];
    for entry in std::fs::read_dir(root.join("gateway")).expect("the gateway directory") {
        let path = entry.expect("a directory entry").path();
        if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let text = std::fs::read_to_string(&p).expect("a gateway source file");
            (p.display().to_string(), text)
        })
        .collect()
}

#[test]
fn the_scan_reads_every_source_file_of_the_gateway() {
    let seen = gateway_sources();
    for name in [
        "crossing.rs",
        "lease.rs",
        "librarian.rs",
        "rate.rs",
        "reviewer.rs",
        "roles.rs",
        "rooms.rs",
        "delivery.rs",
        "workers.rs",
    ] {
        assert!(
            seen.iter().any(|(p, _)| p.ends_with(name)),
            "{name} is not scanned"
        );
    }
}

/// The public methods in the gateway's source files (tests excluded).
fn public_methods() -> Vec<String> {
    let mut names = Vec::new();
    for (_, src) in gateway_sources() {
        for line in src.lines() {
            if let Some(rest) = line.strip_prefix("    pub fn ") {
                names.push(rest.split(['(', '<']).next().unwrap_or("").to_string());
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

#[test]
fn every_public_method_of_the_gateway_is_classified() {
    let mut known: Vec<&str> = GUARDED.iter().map(|(n, _)| *n).collect();
    known.extend(OWN_RECORD.iter().map(|(n, _)| *n));
    // `shared` and `anchored` belong to ClaimSpec, a builder, not to operations
    known.extend(["shared", "anchored"]);
    let unclassified: Vec<String> = public_methods()
        .into_iter()
        .filter(|m| !known.contains(&m.as_str()))
        .collect();
    assert!(
        unclassified.is_empty(),
        "classify these new public methods as GUARDED (with a refusal check) or OWN_RECORD (with a reason): {unclassified:?}"
    );
    let gone: Vec<&&str> = known
        .iter()
        .filter(|k| !public_methods().iter().any(|m| m == **k))
        .collect();
    assert!(gone.is_empty(), "listed but not in the source: {gone:?}");
}

#[test]
fn under_a_guard_that_refuses_everything_no_guarded_method_succeeds_or_touches_the_store() {
    let guard = Rc::new(SpyGuard::denying(&["r", "", "dm.alice.bob"]));
    let g = Gateway::new(SpyStore::new(), FixedClock(1), guard);
    let me = alice();
    let leaks: Vec<&str> = GUARDED
        .iter()
        .filter(|(_, call)| !call(&g, &me))
        .map(|(name, _)| *name)
        .collect();
    assert!(leaks.is_empty(), "not refused by the guard: {leaks:?}");
    assert_eq!(g.store.calls.get(), 0, "a refused call reached the store");
}

#[test]
fn a_reviewer_is_still_refused_by_the_guard_for_both_review_operations() {
    // the role lets a reviewer review; the guard still decides (rooms "" stands for an operation with no room)
    let guard = Rc::new(SpyGuard::denying(&[""]));
    let g = Gateway::new(SpyStore::new(), FixedClock(1), guard.clone());
    g.store.policy_set("role:rev", Some("reviewer"), 1).unwrap();
    let rev = Principal::unattributed("rev");
    assert!(refused(g.review_next(&rev)));
    assert!(refused(g.review_verdict(
        &rev,
        1,
        &crate::domain::Verdict::Allow
    )));
    let seen = guard.seen.borrow();
    assert!(seen.contains(&Operation::ReviewNext), "{seen:?}");
    assert!(
        seen.contains(&Operation::ReviewVerdict { id: 1 }),
        "{seen:?}"
    );
}

#[test]
fn the_guard_sees_every_guarded_method_exactly_once_per_call() {
    let guard = Rc::new(SpyGuard::allowing());
    let g = Gateway::new(SpyStore::new(), FixedClock(1), guard.clone());
    let me = alice();
    for (name, call) in GUARDED {
        // The two review operations are refused by the role (rule `reviewer`) before the guard is asked unless the
        // caller is a reviewer; tests/reviewer.rs proves both the role and the guard for them.
        if matches!(*name, "review_next" | "review_verdict") {
            continue;
        }
        let before = guard.seen.borrow().len();
        let _ = call(&g, &me);
        assert!(
            guard.seen.borrow().len() > before,
            "{name} did not ask the guard"
        );
    }
}
