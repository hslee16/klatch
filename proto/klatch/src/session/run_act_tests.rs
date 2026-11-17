//! The member-side forms of the admin agent's run and anchor acts, and of the housekeeping `prune` (#522):
//! `role_act` with `run-create`, `run-end` and `anchor-done` (admin agent only), and the plain method `prune`
//! (every member's, as at session start). The acts that need a server-side input nobody designed (`reap-anchors`,
//! and a `run-end` archive file) are not on the wire.

use super::role_tests::{answer, call, gateway, rule_of, who};
use crate::domain::MEMBER_RETENTION_SECS;
use crate::gateway::{AllowAll, ClaimSpec, FixedClock, ForbidFloors, Gateway};
use crate::store::SqliteStore;
use crate::store::areas::*;
use serde_json::json;

fn act(
    g: &Gateway<SqliteStore>,
    member: &str,
    params: serde_json::Value,
) -> Vec<crate::protocol::ServerFrame> {
    call(g, member, "role_act", params)
}

#[test]
fn the_admin_agent_opens_and_ends_a_run_over_the_wire() {
    let g = gateway();
    let f = act(&g, "ada", json!({"act":"run-create","run":"r1"}));
    assert_eq!(answer(&f), Ok(json!({})));
    let f = call(
        &g,
        "alice",
        "post",
        json!({"room":"run.r1.work","body":"in the run"}),
    );
    assert!(
        answer(&f).is_ok(),
        "the run room is usable once the run is open: {f:?}"
    );
    let f = act(&g, "ada", json!({"act":"run-end","run":"r1"}));
    assert_eq!(answer(&f), Ok(json!({"rooms":1,"messages":1})));
    let f = call(
        &g,
        "alice",
        "post",
        json!({"room":"run.r1.work","body":"again"}),
    );
    assert_eq!(rule_of(&f).as_deref(), Some("run-unknown"));
}

#[test]
fn a_run_id_is_one_to_thirty_two_safe_characters() {
    let g = gateway();
    let longest = "a".repeat(32);
    let too_long = "a".repeat(33);
    let f = act(&g, "ada", json!({"act":"run-create","run":longest}));
    assert!(answer(&f).is_ok(), "32 characters are allowed: {f:?}");
    for bad in [too_long.as_str(), "", "a.b", "a b", "a/b"] {
        let f = act(&g, "ada", json!({"act":"run-create","run":bad}));
        assert_eq!(answer(&f), Err("invalid".into()), "{bad:?}");
    }
    let f = act(&g, "ada", json!({"act":"run-create","run":"ok-1_x"}));
    assert!(answer(&f).is_ok());
}

#[test]
fn opening_a_run_twice_and_ending_one_that_is_not_open_fail_plainly() {
    let g = gateway();
    act(&g, "ada", json!({"act":"run-create","run":"r1"}));
    let f = act(&g, "ada", json!({"act":"run-create","run":"r1"}));
    assert_eq!(answer(&f), Err("invalid".into()));
    let f = act(&g, "ada", json!({"act":"run-end","run":"nope"}));
    assert_eq!(answer(&f), Err("not_found".into()));
    let f = act(&g, "ada", json!({"act":"run-end","run":"bad id"}));
    assert_eq!(answer(&f), Err("invalid".into()));
}

#[test]
fn ending_a_run_removes_only_that_runs_rooms() {
    let g = gateway();
    act(&g, "ada", json!({"act":"run-create","run":"one"}));
    act(&g, "ada", json!({"act":"run-create","run":"two"}));
    call(&g, "alice", "post", json!({"room":"run.one.a","body":"x"}));
    call(&g, "alice", "post", json!({"room":"run.two.a","body":"y"}));
    call(&g, "alice", "post", json!({"room":"r","body":"outside"}));
    let f = act(&g, "ada", json!({"act":"run-end","run":"one"}));
    assert_eq!(answer(&f), Ok(json!({"rooms":1,"messages":1})));
    let f = call(&g, "alice", "history", json!({"room":"run.two.a"}));
    assert_eq!(answer(&f).unwrap()["messages"].as_array().unwrap().len(), 1);
    let f = call(&g, "alice", "history", json!({"room":"r"}));
    assert_eq!(answer(&f).unwrap()["messages"].as_array().unwrap().len(), 2);
}

#[test]
fn a_run_end_over_the_wire_never_writes_an_archive_file() {
    let g = gateway();
    act(&g, "ada", json!({"act":"run-create","run":"r1"}));
    call(&g, "alice", "post", json!({"room":"run.r1.a","body":"x"}));
    let dir = std::env::temp_dir().join(format!("klatch-no-archive-{}", std::process::id()));
    let f = act(
        &g,
        "ada",
        json!({"act":"run-end","run":"r1","archive":dir.to_string_lossy(),"path":dir.to_string_lossy()}),
    );
    assert!(answer(&f).is_ok(), "{f:?}");
    assert!(
        !dir.exists(),
        "a wire param must not make the server write a file"
    );
}

#[test]
fn the_admin_agent_reports_an_anchor_done_over_the_wire() {
    let g = gateway();
    let spec = ClaimSpec::new("src/a.rs", 600, "work").anchored("gh:pr:o/r/1");
    g.claim(&who("alice"), "r", &spec).unwrap();
    let other = ClaimSpec::new("src/b.rs", 600, "work").anchored("gh:pr:o/r/2");
    g.claim(&who("alice"), "r", &other).unwrap();
    let f = act(
        &g,
        "ada",
        json!({"act":"anchor-done","anchor":"gh:pr:o/r/1"}),
    );
    assert_eq!(answer(&f), Ok(json!({"released":1})));
    let f = call(&g, "alice", "claims", json!({"room":"r"}));
    let claims = answer(&f).unwrap()["claims"].clone();
    assert_eq!(claims.as_array().unwrap().len(), 1);
    assert_eq!(claims[0]["path"], "src/b.rs");
    let f = act(
        &g,
        "ada",
        json!({"act":"anchor-done","anchor":"gh:pr:o/r/9"}),
    );
    assert_eq!(answer(&f), Ok(json!({"released":0})));
}

#[test]
fn an_anchor_is_one_to_eighty_safe_characters() {
    let g = gateway();
    let longest = "a".repeat(80);
    let too_long = "a".repeat(81);
    let f = act(&g, "ada", json!({"act":"anchor-done","anchor":longest}));
    assert!(answer(&f).is_ok(), "80 characters are allowed: {f:?}");
    for bad in [too_long.as_str(), "", "a b", "a;b", "a\\b"] {
        let f = act(&g, "ada", json!({"act":"anchor-done","anchor":bad}));
        assert_eq!(answer(&f), Err("invalid".into()), "{bad:?}");
    }
}

#[test]
fn the_new_acts_need_their_parameters() {
    let g = gateway();
    for params in [
        json!({"act":"run-create"}),
        json!({"act":"run-end"}),
        json!({"act":"anchor-done"}),
        json!({"act":"run-create","run":5}),
        json!({"act":"anchor-done","anchor":["x"]}),
    ] {
        let f = act(&g, "ada", params.clone());
        assert_eq!(answer(&f), Err("invalid_params".into()), "{params}");
    }
}

#[test]
fn an_ordinary_member_is_refused_every_new_act_and_nothing_changes() {
    let g = gateway();
    for params in [
        json!({"act":"run-create","run":"r1"}),
        json!({"act":"run-end","run":"r1"}),
        json!({"act":"anchor-done","anchor":"gh:pr:o/r/1"}),
    ] {
        let f = act(&g, "alice", params.clone());
        assert_eq!(answer(&f), Err("refused".into()), "{params}");
        assert_eq!(rule_of(&f).as_deref(), Some("admin-scope"), "{params}");
    }
    let f = call(
        &g,
        "alice",
        "post",
        json!({"room":"run.r1.work","body":"x"}),
    );
    assert_eq!(
        rule_of(&f).as_deref(),
        Some("run-unknown"),
        "the run was never opened"
    );
}

#[test]
fn the_reviewer_is_refused_every_new_act() {
    let g = gateway();
    for params in [
        json!({"act":"run-create","run":"r1"}),
        json!({"act":"run-end","run":"r1"}),
        json!({"act":"anchor-done","anchor":"gh:pr:o/r/1"}),
    ] {
        let f = act(&g, "rev", params.clone());
        assert_eq!(answer(&f), Err("refused".into()), "{params}");
        assert_eq!(rule_of(&f).as_deref(), Some("reviewer"), "{params}");
    }
}

#[test]
fn a_refused_act_is_audited_and_a_run_act_of_the_admin_agent_names_it() {
    let g = gateway();
    act(&g, "alice", json!({"act":"run-create","run":"r1"}));
    act(&g, "ada", json!({"act":"run-create","run":"r2"}));
    let f = act(&g, "ada", json!({"act":"audit","limit":10}));
    let records = answer(&f).unwrap()["records"].clone();
    let text = records.to_string();
    assert!(text.contains("run-create"), "{text}");
    assert!(text.contains("by ada"), "{text}");
    assert!(
        text.contains("refused"),
        "the refusal of alice is audited: {text}"
    );
}

#[test]
fn reap_anchors_and_a_run_archive_are_not_wire_acts() {
    let g = gateway();
    for name in [
        "reap-anchors",
        "reap",
        "run-archive",
        "runs",
        "hook_runs",
        "prune",
    ] {
        let f = act(&g, "ada", json!({"act":name,"dir":"/tmp","run":"r1"}));
        assert_eq!(answer(&f), Err("unknown_act".into()), "{name}");
    }
}

fn members_gateway(now: i64) -> Gateway<SqliteStore> {
    Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        FixedClock(now),
        ForbidFloors::new(AllowAll),
    )
    .for_members()
}

#[test]
fn prune_removes_members_not_seen_for_the_retention_and_keeps_the_rest() {
    let now = 1_000_000;
    let g = members_gateway(now);
    let store = g.store_for_tests();
    store
        .join("r", "stale", now - MEMBER_RETENTION_SECS - 1)
        .unwrap();
    store
        .join("r", "edge", now - MEMBER_RETENTION_SECS)
        .unwrap();
    store.join("r", "fresh", now - 10).unwrap();
    let f = call(&g, "alice", "prune", json!({}));
    assert_eq!(answer(&f), Ok(json!({"removed":1})));
    let f = call(&g, "alice", "who", json!({"room":"r"}));
    let names: Vec<String> = answer(&f).unwrap()["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap().to_string())
        .collect();
    assert!(
        names.contains(&"edge".to_string()) && names.contains(&"fresh".to_string()),
        "{names:?}"
    );
    assert!(!names.contains(&"stale".to_string()), "{names:?}");
}

#[test]
fn prune_uses_the_fixed_retention_and_ignores_any_age_a_caller_names() {
    let now = 1_000_000;
    let g = members_gateway(now);
    g.store_for_tests().join("r", "recent", now - 5).unwrap();
    let f = call(
        &g,
        "alice",
        "prune",
        json!({"older_than":1,"retention":1,"seconds":1}),
    );
    assert_eq!(answer(&f), Ok(json!({"removed":0})));
    let f = call(&g, "alice", "who", json!({"room":"r"}));
    assert_eq!(answer(&f).unwrap()["members"].as_array().unwrap().len(), 1);
}

#[test]
fn prune_is_every_members_housekeeping_but_not_a_librarians_or_reviewers() {
    let g = gateway();
    let f = call(&g, "alice", "prune", json!({}));
    assert!(answer(&f).is_ok(), "{f:?}");
    let f = call(&g, "rev", "prune", json!({}));
    assert_eq!(answer(&f), Err("refused".into()));
    assert_eq!(rule_of(&f).as_deref(), Some("reviewer"));
}
