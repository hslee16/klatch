//! Zero trust in code (#161, R35): no operation reaches the store without the guard. Every public method of the
//! gateway is listed in one of the area files under `zero_trust/`, as guarded or as acting only on the caller's own
//! record, and a test fails when a new method is not listed, so a new operation cannot skip the decision.
//!
//! How to classify a new public method of the gateway (#1067):
//!
//! 1. Pick the area file of what the method is about: `messages`, `claims`, `crossings_review`, `delivery_view`,
//!    `keys_credentials` or `roles_admin`. Add the entry there, on one line with a trailing comma, so two pull
//!    requests that add methods to the same file merge by keeping both lines.
//! 2. A method that takes a room or acts on anyone but the caller goes in that file's `GUARDED` list with a call that
//!    must be refused under a guard that refuses everything (the closure returns `refused(...)`).
//! 3. A method that touches only the caller's own record, builds the gateway, or looks up policy goes in
//!    `OWN_RECORD` with a reason. Then list it in exactly one of `NO_PRINCIPAL` (it takes no principal) or
//!    `AFTER_END` (it takes the caller's principal: `None` if an ended credential is refused, `Some(reason)` if the
//!    end of a worker's session needs it).
//! 4. Keep every list of a file to its own area: a name in two lists of the sort (GUARDED and OWN_RECORD) or in two
//!    area files fails the test, and so does an entry that names no method.
//!
//! The area files must stay under 600 lines (`tests/hub_files_stay_split.rs`); a seventh area is a new file and a new
//! `mod` line and a new line in each `*_lists` function below.

use super::*;
use crate::gateway::ClaimSpec;

mod claims;
mod crossings_review;
mod delivery_view;
mod keys_credentials;
mod messages;
mod roles_admin;

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
/// The areas, in the order the lists run: the guarded calls of one area run before those of the next.
const AREAS: [&str; 6] = [
    "messages",
    "claims",
    "crossings_review",
    "delivery_view",
    "keys_credentials",
    "roles_admin",
];

/// Every guarded method of every area, with the name of its area.
fn guarded_by_area() -> Vec<(&'static str, &'static str, Call)> {
    let lists: [&[(&str, Call)]; 6] = [
        messages::GUARDED,
        claims::GUARDED,
        crossings_review::GUARDED,
        delivery_view::GUARDED,
        keys_credentials::GUARDED,
        roles_admin::GUARDED,
    ];
    let mut out = Vec::new();
    for (area, list) in AREAS.iter().zip(lists) {
        out.extend(list.iter().map(|(n, c)| (*area, *n, *c)));
    }
    out
}

fn guarded() -> Vec<(&'static str, Call)> {
    guarded_by_area()
        .into_iter()
        .map(|(_, n, c)| (n, c))
        .collect()
}

/// Every own-record method of every area, with the name of its area.
fn own_record_by_area() -> Vec<(&'static str, &'static str, &'static str)> {
    let lists: [&[(&str, &str)]; 6] = [
        messages::OWN_RECORD,
        claims::OWN_RECORD,
        crossings_review::OWN_RECORD,
        delivery_view::OWN_RECORD,
        keys_credentials::OWN_RECORD,
        roles_admin::OWN_RECORD,
    ];
    let mut out = Vec::new();
    for (area, list) in AREAS.iter().zip(lists) {
        out.extend(list.iter().map(|(n, r)| (*area, *n, *r)));
    }
    out
}

fn own_record() -> Vec<(&'static str, &'static str)> {
    own_record_by_area()
        .into_iter()
        .map(|(_, n, r)| (n, r))
        .collect()
}

fn no_principal() -> Vec<&'static str> {
    [
        messages::NO_PRINCIPAL,
        claims::NO_PRINCIPAL,
        crossings_review::NO_PRINCIPAL,
        delivery_view::NO_PRINCIPAL,
        keys_credentials::NO_PRINCIPAL,
        roles_admin::NO_PRINCIPAL,
    ]
    .concat()
}

fn after_end() -> Vec<(&'static str, OwnCall, Option<&'static str>)> {
    [
        messages::AFTER_END,
        claims::AFTER_END,
        crossings_review::AFTER_END,
        delivery_view::AFTER_END,
        keys_credentials::AFTER_END,
        roles_admin::AFTER_END,
    ]
    .concat()
}

/// The `.rs` files in `dir` and in its subdirectories other than the tests (the board is split by section, #1066).
fn collect_sources(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("the gateway directory") {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "tests") {
                collect_sources(&path, files);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
}

/// The source files of the gateway, found by reading the directory so that a new file cannot be left out of the
/// scan (#161): `gateway.rs` and every file beside it, tests excluded.
fn gateway_sources() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/gateway");
    let mut files = vec![root.join("gateway.rs")];
    collect_sources(&root.join("gateway"), &mut files);
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
        "digest.rs",
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
    let mut known: Vec<&str> = guarded().iter().map(|(n, _)| *n).collect();
    known.extend(own_record().iter().map(|(n, _)| *n));
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
    // a method is listed once: not in GUARDED and OWN_RECORD, and not in two areas (#1067)
    let mut seen: Vec<&str> = Vec::new();
    let mut twice: Vec<String> = Vec::new();
    let all = guarded_by_area()
        .into_iter()
        .map(|(area, name, _)| (area, name, "GUARDED"))
        .chain(
            own_record_by_area()
                .into_iter()
                .map(|(area, name, _)| (area, name, "OWN_RECORD")),
        );
    for (area, name, list) in all {
        if seen.contains(&name) {
            twice.push(format!("{name} (again in {area}, {list})"));
        }
        seen.push(name);
    }
    assert!(twice.is_empty(), "listed more than once: {twice:?}");
}

#[test]
fn under_a_guard_that_refuses_everything_no_guarded_method_succeeds_or_touches_the_store() {
    let guard = Rc::new(SpyGuard::denying(&["r", "", "dm.alice.bob"]));
    let g = Gateway::new(SpyStore::new(), FixedClock(1), guard);
    let me = alice();
    let leaks: Vec<&str> = guarded()
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
    for (name, call) in &guarded() {
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

/// Every own-record method that takes the caller's principal (#835): a verified principal whose credential ended
/// (revoked, expired) is refused (`None`), unless the end of a worker's session needs the method (`Some(reason)`).
/// A new own-record method that takes a principal must be added here, and the tests below prove each row.
type OwnCall = fn(&Gateway<SpyStore>, &Principal) -> Result<()>;

#[test]
fn every_own_record_method_is_classified_for_a_principal_whose_credential_ended() {
    let in_table: Vec<&str> = after_end().iter().map(|(n, _, _)| *n).collect();
    for (name, _) in &own_record() {
        let no_principal = no_principal().contains(name);
        let with_principal = in_table.contains(name);
        assert!(
            no_principal != with_principal,
            "own-record method {name}: list it in exactly one of NO_PRINCIPAL or AFTER_END (#835)"
        );
    }
    let strays: Vec<&str> = no_principal()
        .into_iter()
        .chain(in_table.iter().copied())
        .filter(|n| !own_record().iter().any(|(o, _)| o == n))
        .collect();
    assert!(
        strays.is_empty(),
        "classified but not in OWN_RECORD: {strays:?}"
    );
}

/// How a credential ends, so every row is proven for each way.
#[derive(Clone, Copy, Debug)]
enum Ending {
    SessionEnd,
    AdminRevoke,
    Expiry,
}

fn worker_spec(max_life: Option<i64>) -> CredentialSpec<'static> {
    CredentialSpec {
        member: "w-1",
        clearance: "internal",
        ttl: 600,
        max_life,
        parent: None,
    }
}

fn ended_worker(how: Ending) -> (Gateway<SpyStore>, Principal) {
    let (g, t) = timed();
    let token = g
        .admin_issue_credential(&admin(), &worker_spec(Some(600)))
        .unwrap();
    let who = g.verify_credential("w-1", &token).unwrap();
    match how {
        Ending::SessionEnd => assert!(g.end_credential(&who).unwrap()),
        Ending::AdminRevoke => g.admin_revoke_credential(&admin(), "w-1").unwrap(),
        Ending::Expiry => t.set(t.get() + 601),
    }
    (g, who)
}

#[test]
fn a_live_credential_may_call_every_own_record_method() {
    for (name, call, _) in &after_end() {
        let (g, t) = timed();
        let token = g
            .admin_issue_credential(&admin(), &worker_spec(None))
            .unwrap();
        let who = g.verify_credential("w-1", &token).unwrap();
        t.set(t.get() + 1);
        let r = call(&g, &who);
        assert!(
            !matches!(r, Err(Error::Refused { .. })),
            "{name} refused a live credential: {r:?}"
        );
    }
}

#[test]
fn every_own_record_method_refuses_an_ended_credential_unless_it_is_listed_with_a_reason() {
    for how in [Ending::SessionEnd, Ending::AdminRevoke, Ending::Expiry] {
        for (name, call, allowed) in &after_end() {
            let (g, who) = ended_worker(how);
            assert_after_end(name, how, call(&g, &who), allowed.is_some());
        }
    }
}

/// After the end, an own-record call is refused with the rule `credential`, unless it is listed as allowed.
fn assert_after_end(name: &str, how: Ending, r: Result<()>, allowed: bool) {
    if allowed {
        assert!(
            !matches!(r, Err(Error::Refused { .. })),
            "{name} is listed as allowed after the end but was refused after {how:?}: {r:?}"
        );
        return;
    }
    match r {
        Err(Error::Refused { rule, .. }) => {
            assert_eq!(rule, "credential", "{name} after {how:?}")
        }
        other => panic!("{name} was not refused after {how:?}: {other:?}"),
    }
}

#[test]
fn after_the_end_only_the_session_end_hook_run_is_recorded() {
    let (g, who) = ended_worker(Ending::Expiry);
    for event in ["Stop", "SessionStart", "PostToolUse"] {
        assert!(is_refused(g.record_hook_run(&who, event, 0)), "{event}");
    }
    g.record_hook_run(&who, "SessionEnd", 0).unwrap();
}

#[test]
fn the_session_end_sequence_still_frees_claims_and_ends_the_credential_after_an_idle_expiry() {
    let (g, t) = timed();
    let token = g
        .admin_issue_credential(&admin(), &worker_spec(Some(600)))
        .unwrap();
    let who = g.verify_credential("w-1", &token).unwrap();
    g.join(&who, "r").unwrap();
    g.claim(&who, "r", &ClaimSpec::new("p", 3_000, "")).unwrap();
    t.set(t.get() + 601);
    assert!(g.release_all(&who).unwrap() >= 1, "the claim was not freed");
    g.record_hook_run(&who, "SessionEnd", 0).unwrap();
    g.end_credential(&who).unwrap();
    assert!(g.claims(&admin(), "r").unwrap().is_empty());
}

#[test]
fn a_verified_principal_proven_some_other_way_and_a_typed_name_are_not_touched_by_the_own_record_check()
 {
    let (g, t) = timed();
    let token = g
        .admin_issue_credential(&admin(), &worker_spec(None))
        .unwrap();
    let _ = g.verify_credential("w-1", &token).unwrap();
    t.set(t.get() + 700);
    let peer = Principal::verified("peer-1");
    let typed = Principal::unattributed("w-1");
    for (name, call, _) in &after_end() {
        if *name == "end_credential" {
            continue;
        }
        for who in [&peer, &typed] {
            let r = call(&g, who);
            assert!(
                !matches!(r, Err(Error::Refused { .. })),
                "{name} refused {who:?}: {r:?}"
            );
        }
    }
}
