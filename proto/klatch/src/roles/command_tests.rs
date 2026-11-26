//! `klatch act` and `klatch review` reach the gateway (#725): each command calls its own transport method once, and the
//! read commands (`audit`, `refusals`) and `review` are refused for a caller without the role, which an empty
//! function body would hide (it would answer `Ok` without asking).

use super::*;
use klatch::gateway::FixedClock;
use klatch::gateway::areas::*;

/// Two gateways over one database, as in production: the human's (the terminal, no role needed) and the members'.
struct World {
    gateway: Rc<Gateway<SqliteStore>>,
    human: Gateway<SqliteStore>,
    dir: crate::test_dir::TestDir,
}

impl World {
    fn new(tag: &str) -> World {
        let dir = crate::test_dir::TestDir::new(&format!("roles-{tag}"));
        let open = || SqliteStore::open(&dir.join("t.db"), &klatch::store::insecure_test_key());
        let gateway = Gateway::new(open().unwrap(), FixedClock(1_000), AllowAll).for_members();
        let human = Gateway::new(open().unwrap(), FixedClock(1_000), AllowAll);
        let w = World {
            gateway: Rc::new(gateway),
            human,
            dir,
        };
        w.as_member("alice").join("r").unwrap();
        w
    }
    /// A member of the same database whose clock reads `now`: the same store, a later moment.
    fn later(&self, member: &str, now: i64) -> Direct<SqliteStore> {
        let store =
            SqliteStore::open(&self.dir.join("t.db"), &klatch::store::insecure_test_key()).unwrap();
        let gateway = Gateway::new(store, FixedClock(now), AllowAll).for_members();
        Direct::new(Rc::new(gateway), Principal::unattributed(member))
    }
    fn open_runs(&self) -> usize {
        self.human.admin_runs(&World::human()).unwrap().len()
    }
    fn as_member(&self, member: &str) -> Direct<SqliteStore> {
        Direct::new(self.gateway.clone(), Principal::unattributed(member))
    }
    fn human() -> Principal {
        Principal::unattributed("admin")
    }
    fn admin_agent(&self) -> Direct<SqliteStore> {
        self.human
            .admin_set_admin_agent(&World::human(), "ada", true)
            .unwrap();
        self.as_member("ada")
    }
    /// How many times the human's audit trail holds this act on this target.
    fn audited(&self, action: &str, target: &str) -> usize {
        self.gateway
            .admin_audit(&Principal::unattributed("ada"), 100)
            .unwrap()
            .iter()
            .filter(|a| a.action == action && a.target == target)
            .count()
    }
}

#[test]
fn each_act_command_makes_its_one_admin_call() {
    let w = World::new("acts");
    let t = w.admin_agent();
    let run = |cmd| act(&t, cmd).unwrap();
    run(ActCmd::Close { room: "r".into() });
    assert_eq!(w.audited("close", "r"), 1);
    assert_eq!(w.audited("reopen", "r"), 0, "close is not reopen");
    run(ActCmd::Reopen { room: "r".into() });
    assert_eq!(w.audited("reopen", "r"), 1);
    assert_eq!(w.audited("close", "r"), 1);
    run(ActCmd::Remove {
        member: "alice".into(),
        room: "r".into(),
    });
    assert_eq!(w.audited("remove", "r/alice"), 1);
    run(ActCmd::Allow {
        member: "alice".into(),
        room: "r".into(),
    });
    assert_eq!(w.audited("allow", "r/alice"), 1);
    assert_eq!(w.audited("remove", "r/alice"), 1);
    run(ActCmd::Label {
        room: "r".into(),
        label: "internal".into(),
    });
    assert_eq!(w.audited("label", "r=internal"), 1);
    run(ActCmd::Clear {
        member: "alice".into(),
        label: "private".into(),
    });
    assert_eq!(w.gateway.clearance_of("alice").unwrap(), "private");
}

#[test]
fn a_close_really_closes_the_room_and_a_reopen_opens_it() {
    let w = World::new("closed");
    let t = w.admin_agent();
    act(&t, ActCmd::Close { room: "r".into() }).unwrap();
    assert!(w.as_member("alice").post("r", "x").is_err());
    act(&t, ActCmd::Reopen { room: "r".into() }).unwrap();
    assert!(w.as_member("alice").post("r", "x").is_ok());
}

#[test]
fn audit_and_refusals_ask_the_gateway_and_are_refused_without_a_role() {
    let w = World::new("reads");
    let nobody = w.as_member("nobody");
    assert!(act(&nobody, ActCmd::Audit { limit: 5 }).is_err());
    assert!(act(&nobody, ActCmd::Refusals { limit: 5 }).is_err());
    let t = w.admin_agent();
    assert!(act(&t, ActCmd::Audit { limit: 5 }).is_ok());
    assert!(act(&t, ActCmd::Refusals { limit: 5 }).is_ok());
}

#[test]
fn an_act_by_a_member_without_the_role_changes_nothing() {
    let w = World::new("norole");
    let nobody = w.as_member("nobody");
    assert!(act(&nobody, ActCmd::Close { room: "r".into() }).is_err());
    assert!(w.as_member("alice").post("r", "x").is_ok());
}

#[test]
fn review_commands_ask_the_gateway_and_are_refused_without_the_role() {
    let w = World::new("review");
    let nobody = w.as_member("nobody");
    assert!(review(&nobody, ReviewCmd::Next).is_err());
    let verdict = |id| ReviewCmd::Verdict {
        id,
        verdict: VerdictCmd::Allow,
    };
    assert!(review(&nobody, verdict(1)).is_err());
    w.human
        .admin_set_reviewer(&World::human(), "rita", true)
        .unwrap();
    let rita = w.as_member("rita");
    assert!(review(&rita, ReviewCmd::Next).is_ok(), "an empty queue");
    assert!(
        review(&rita, verdict(999)).is_err(),
        "a verdict on an item that is not waiting is an error, not a silent success"
    );
}

#[test]
fn run_create_and_run_end_open_and_end_a_run_as_the_admin_agent() {
    let w = World::new("runs");
    let t = w.admin_agent();
    act(&t, ActCmd::RunCreate { run: "r1".into() }).unwrap();
    assert_eq!(w.audited("run-create", "r1"), 1);
    assert_eq!(w.open_runs(), 1);
    assert!(
        act(&t, ActCmd::RunCreate { run: "r1".into() }).is_err(),
        "a run that is open cannot be opened again"
    );
    w.as_member("alice").post("run.r1.chat", "x").unwrap();
    act(&t, ActCmd::RunEnd { run: "r1".into() }).unwrap();
    assert_eq!(w.audited("run-end", "r1"), 1);
    assert_eq!(w.open_runs(), 0);
    assert!(
        w.as_member("alice").post("run.r1.chat", "y").is_err(),
        "the rooms of an ended run are gone and refused"
    );
    assert!(
        act(&t, ActCmd::RunEnd { run: "r1".into() }).is_err(),
        "a run that is not open cannot be ended"
    );
}

#[test]
fn run_acts_by_a_member_without_the_role_change_nothing() {
    let w = World::new("runs-norole");
    let nobody = w.as_member("nobody");
    assert!(act(&nobody, ActCmd::RunCreate { run: "r1".into() }).is_err());
    assert_eq!(w.open_runs(), 0);
    let t = w.admin_agent();
    act(&t, ActCmd::RunCreate { run: "r1".into() }).unwrap();
    assert!(act(&nobody, ActCmd::RunEnd { run: "r1".into() }).is_err());
    assert_eq!(w.open_runs(), 1, "the run is still open");
    assert!(
        act(
            &t,
            ActCmd::RunCreate {
                run: "bad id".into()
            }
        )
        .is_err()
    );
}

#[test]
fn anchor_done_releases_the_claims_anchored_to_that_item_and_no_others() {
    let w = World::new("anchors");
    let alice = w.as_member("alice");
    for (path, anchor) in [("a.rs", "gh:pr:o/r/1"), ("b.rs", "gh:pr:o/r/2")] {
        alice
            .claim("r", &ClaimSpec::new(path, 600, "work").anchored(anchor))
            .unwrap();
    }
    let nobody = w.as_member("nobody");
    let one = || ActCmd::AnchorDone {
        anchor: "gh:pr:o/r/1".into(),
    };
    assert!(act(&nobody, one()).is_err());
    assert_eq!(alice.claims("r").unwrap().len(), 2, "no role, no release");
    act(&w.admin_agent(), one()).unwrap();
    assert_eq!(w.audited("anchor-done", "gh:pr:o/r/1"), 1);
    let left: Vec<String> = alice
        .claims("r")
        .unwrap()
        .into_iter()
        .map(|c| c.path)
        .collect();
    assert_eq!(left, vec!["b.rs".to_string()]);
}

#[test]
fn prune_removes_the_members_not_seen_for_the_retention_for_any_member() {
    let w = World::new("prune");
    let late = w.later("bob", 1_000 + klatch::domain::MEMBER_RETENTION_SECS + 10);
    late.join("r").unwrap();
    act(&late, ActCmd::Prune).unwrap();
    let names: Vec<String> = late.who("r").unwrap().into_iter().map(|m| m.name).collect();
    assert_eq!(
        names,
        vec!["bob".to_string()],
        "alice was not seen for a day"
    );
}

#[test]
fn the_audit_listing_leaves_out_the_forwarded_note_and_says_how_many_it_hid() {
    use klatch::session::Backend;
    let w = World::new("audit-note");
    let t = w.admin_agent();
    run_close(&t);
    for _ in 0..3 {
        w.gateway
            .note_forwarded_ignored("requests=1 x-real-ip=1")
            .unwrap();
    }
    let lines = audit_listing(&t, 20).unwrap();
    assert!(lines.iter().any(|l| l.contains("\tclose\t")), "{lines:#?}");
    assert!(
        lines.iter().all(|l| !l.contains("\tforwarded-ignored\t")),
        "{lines:#?}"
    );
    assert_eq!(
        lines.last().unwrap(),
        "3 forwarded-ignored rows hidden (operational notes; the human's `klatch admin audit --action forwarded-ignored` shows them)"
    );
}

fn run_close(t: &Direct<SqliteStore>) {
    act(t, ActCmd::Close { room: "r".into() }).unwrap();
}
