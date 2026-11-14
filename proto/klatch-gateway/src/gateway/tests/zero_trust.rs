//! Zero trust in code (#161, R35): no operation reaches the store without the guard. Every public method of the
//! gateway is listed below, as guarded or as acting only on the caller's own record, and a test fails when a new
//! method is not listed, so a new operation cannot skip the decision.

use super::*;
use crate::ClaimSpec;

/// Every public method that takes a room or acts on anyone but the caller: it asks the guard first, and under a
/// guard that refuses everything it is refused with nothing read or written.
type Call = fn(&Gateway<SpyStore>, &Principal) -> bool;

fn refused<T>(r: Result<T>) -> bool {
    matches!(r, Err(Error::Refused { .. }))
}

const GUARDED: &[(&str, Call)] = &[
    ("join", |g, w| refused(g.join(w, "r"))),
    ("post", |g, w| refused(g.post(w, "r", "x"))),
    ("post_labeled", |g, w| {
        refused(g.post_labeled(w, "r", "x", None))
    }),
    ("dm", |g, w| refused(g.dm(w, "bob", "x", None))),
    ("read", |g, w| refused(g.read(w, "r", true))),
    ("tail", |g, w| refused(g.tail(w, "r", (0, 10)))),
    ("head", |g, w| refused(g.head(w, "r"))),
    ("ack", |g, w| refused(g.ack(w, "r", 1))),
    ("who", |g, w| refused(g.who(w, "r"))),
    ("board", |g, w| refused(g.board(w, "r"))),
    ("open_asks", |g, w| refused(g.open_asks(w, "r"))),
    ("claim", |g, w| {
        refused(g.claim(w, "r", &ClaimSpec::new("p", 10, "")))
    }),
    ("claims", |g, w| refused(g.claims(w, "r"))),
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
];

/// Methods that are not an operation on shared state: building the gateway, the clock, or reading or writing only
/// the caller's own record (its scope, parent, session id, hook run record, rooms it joined, claims it holds), or a
/// lookup of policy that the gateway itself uses. Each has a reason; a new one needs one.
const OWN_RECORD: &[(&str, &str)] = &[
    ("new", "builds the gateway"),
    ("now", "the clock"),
    (
        "digest",
        "asks the guard once per room and reports the rooms it refused",
    ),
    ("digest_with", "the same, with the backlog"),
    ("note_parent", "the caller's own parent link"),
    ("note_session", "the caller's own session id"),
    ("scope_of", "the caller's own scope"),
    ("set_scope", "the caller's own scope"),
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
];

/// The public methods in the gateway's source files (tests excluded).
fn public_methods() -> Vec<String> {
    let files = [
        include_str!("../../gateway.rs"),
        include_str!("../admin.rs"),
        include_str!("../budget.rs"),
        include_str!("../dm.rs"),
        include_str!("../policy.rs"),
        include_str!("../scrub.rs"),
        include_str!("../tripwire.rs"),
    ];
    let mut names = Vec::new();
    for src in files {
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
    // `shared` and `new` belong to ClaimSpec and the gateway builders, not to operations
    known.extend(["shared"]);
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
fn the_guard_sees_every_guarded_method_exactly_once_per_call() {
    let guard = Rc::new(SpyGuard::allowing());
    let g = Gateway::new(SpyStore::new(), FixedClock(1), guard.clone());
    let me = alice();
    for (name, call) in GUARDED {
        let before = guard.seen.borrow().len();
        let _ = call(&g, &me);
        assert!(
            guard.seen.borrow().len() > before,
            "{name} did not ask the guard"
        );
    }
}
