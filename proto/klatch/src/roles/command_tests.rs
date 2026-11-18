//! `klatch act` and `klatch review` reach the gateway (#725): each command calls its own transport method once, and the
//! read commands (`audit`, `refusals`) and `review` are refused for a caller without the role, which an empty
//! function body would hide (it would answer `Ok` without asking).

use super::*;
use klatch::gateway::FixedClock;

/// Two gateways over one database, as in production: the human's (the terminal, no role needed) and the members'.
struct World {
    gateway: Rc<Gateway<SqliteStore>>,
    human: Gateway<SqliteStore>,
}

impl World {
    fn new(tag: &str) -> World {
        let dir = std::env::temp_dir().join(format!("klatch-roles-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let open = || SqliteStore::open(&dir.join("t.db"), &klatch::store::insecure_test_key());
        let gateway = Gateway::new(open().unwrap(), FixedClock(1_000), AllowAll).for_members();
        let human = Gateway::new(open().unwrap(), FixedClock(1_000), AllowAll);
        let w = World {
            gateway: Rc::new(gateway),
            human,
        };
        w.as_member("alice").join("r").unwrap();
        w
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
