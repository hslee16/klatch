//! End-to-end tests: run the real binary against a throwaway KLATCH_HOME.
mod common;

use common::*;
use std::process::Command;

#[test]
fn two_scopes_claim_the_same_path_and_one_scope_conflicts_with_itself() {
    let h = Home::new();
    assert!(
        as_scope(&h, "a", "git:a", &["claim", "r", "src/lib.rs"])
            .status
            .success()
    );
    assert!(
        as_scope(&h, "b", "git:b", &["claim", "r", "src/lib.rs"])
            .status
            .success(),
        "another scope may claim it"
    );
    let c = as_scope(&h, "c", "git:a", &["claim", "r", "src/lib.rs"]);
    assert!(!c.status.success(), "the same scope must conflict");
    assert!(
        String::from_utf8_lossy(&c.stderr).contains("claimed by a"),
        "{}",
        String::from_utf8_lossy(&c.stderr)
    );
}

#[test]
fn a_declared_scope_is_remembered_and_the_claims_listing_shows_it() {
    let h = Home::new();
    as_scope(&h, "a", "git:a", &["claim", "r", "p1"]);
    h.ok("a", &["claim", "r", "p2"]); // no --scope this time: the member keeps git:a
    let claims = h.ok("viewer", &["claims", "r"]);
    assert_eq!(claims.matches("scope: git:a").count(), 2, "{claims}");
    let r = h.run("b", &["claim", "r", "p2"]);
    assert!(
        r.status.success(),
        "b has no scope, so it does not collide with git:a: {}",
        String::from_utf8_lossy(&r.stderr)
    );
}

#[test]
fn a_shared_claim_and_an_ask_claim_conflict_across_scopes() {
    let h = Home::new();
    as_scope(&h, "a", "git:a", &["claim", "r", "docs", "--shared"]);
    assert!(
        !as_scope(&h, "b", "git:b", &["claim", "r", "docs/api.md"])
            .status
            .success()
    );
    as_scope(&h, "a", "git:a", &["claim", "r", "ask:3"]);
    assert!(
        !as_scope(&h, "b", "git:b", &["claim", "r", "ask:3"])
            .status
            .success(),
        "an ask is room-wide"
    );
}

#[test]
fn the_scope_command_shows_and_sets_the_scope_and_refuses_a_bad_one() {
    let h = Home::new();
    assert!(h.ok("a", &["scope"]).contains("no scope"), "none yet");
    h.ok("a", &["scope", "bot:grok"]);
    assert!(h.ok("a", &["scope"]).contains("bot:grok"));
    let bad = h.fail("a", &["scope", "has space"]);
    assert!(bad.to_lowercase().contains("scope"), "{bad}");
}

#[test]
fn the_role_command_says_none_for_a_member_without_a_role() {
    let h = Home::new();
    assert_eq!(h.ok("a", &["role"]).trim(), "none");
}

#[test]
fn a_hook_does_not_wait_forever_on_a_standard_input_that_never_closes() {
    let h = Home::new();
    let mut child = Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env_remove("KLATCH_IDENTITY_FILE")
        .env("KLATCH_HOME", &h.0)
        .env("KLATCH_KEY_DIR", h.0.join("keys"))
        .env("KLATCH_NAME", "alice")
        .args(["hook", "--event", "SessionStart"])
        .stdin(std::process::Stdio::piped()) // opened, never written, never closed
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let start = std::time::Instant::now();
    let finished = loop {
        if child.try_wait().unwrap().is_some() {
            break true;
        }
        if start.elapsed().as_secs() >= 8 {
            break false;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    if !finished {
        let _ = child.kill();
    }
    let _ = child.wait();
    assert!(
        finished,
        "the hook hung on a standard input that never closed"
    );
}

/// #632: two members in different scopes held `design-item:66` at once. A work item name is one thing in the whole
/// room, so it conflicts across scopes without `--shared`, and its case or stray spaces do not make it another name.
#[test]
fn a_work_item_name_conflicts_across_scopes_without_shared_and_ignoring_case() {
    let h = Home::new();
    assert!(
        as_scope(&h, "a", "git:a", &["claim", "r", "design-item:66"])
            .status
            .success()
    );
    for other in ["design-item:66", "Design-Item:66", " design-item:66 "] {
        let b = as_scope(&h, "b", "git:b", &["claim", "r", other]);
        assert!(!b.status.success(), "{other:?} must conflict across scopes");
    }
    for other in ["ticket:7", "issue:7", "pr:7", "task:7", "item:7"] {
        assert!(
            as_scope(&h, "a", "git:a", &["claim", "r", other])
                .status
                .success()
        );
        assert!(
            !as_scope(&h, "b", "git:b", &["claim", "r", &other.to_uppercase()])
                .status
                .success(),
            "{other}"
        );
    }
    // a path is still per scope
    assert!(
        as_scope(&h, "a", "git:a", &["claim", "r", "src/ticket:1"])
            .status
            .success()
    );
    assert!(
        as_scope(&h, "b", "git:b", &["claim", "r", "src/ticket:1"])
            .status
            .success()
    );
    // the canonical name releases it, whatever case the holder typed
    as_scope(&h, "a", "git:a", &["release", "r", "DESIGN-ITEM:66"]);
    assert!(
        as_scope(&h, "b", "git:b", &["claim", "r", "design-item:66"])
            .status
            .success()
    );
}
