//! Workers with an issued credential (#121, #120, #161): a launcher writes a key bundle, the worker is handed its
//! path, and from its first action it is a verified member with the clearance the credential carries.
mod common;

use common::*;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::process::Output;

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// Issue a bundle for `member` and return its path.
fn issue(h: &Home, member: &str, clearance: &str, extra: &[&str]) -> PathBuf {
    let out = h.0.join(format!("{member}.bundle"));
    let mut args = vec![
        "worker",
        "issue",
        member,
        "--clearance",
        clearance,
        "--out",
        out.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    let o = h.launcher(&args);
    assert!(o.status.success(), "issue failed: {}", text(&o));
    out
}

#[test]
fn a_launcher_issues_a_bundle_that_only_its_owner_can_read_and_never_prints_the_secrets() {
    let h = Home::new();
    let b = issue(&h, "w-1", "internal", &["--parent", "lead"]);
    assert_eq!(std::fs::metadata(&b).unwrap().mode() & 0o777, 0o600);
    let contents = std::fs::read_to_string(&b).unwrap();
    let o = h.launcher(&["worker", "list"]);
    let shown = text(&o);
    for line in contents
        .lines()
        .filter(|l| l.starts_with("token ") || l.starts_with("store "))
    {
        let secret = line.split(' ').nth(1).unwrap();
        assert!(!shown.contains(secret), "a secret reached the listing");
    }
    assert!(
        shown.contains("w-1") && shown.contains("live") && shown.contains("lead"),
        "{shown}"
    );
}

#[test]
fn a_worker_with_a_bundle_is_a_verified_member_from_its_first_action() {
    let h = Home::new();
    let b = issue(&h, "w-1", "internal", &[]);
    let post = h.worker(&b, &["post", "r", "from the worker"]);
    assert!(post.status.success(), "{}", text(&post));
    let read = h.ok("bob", &["read", "r", "--peek"]);
    assert!(
        read.contains("w-1") && !read.contains("w-1 (unverified)"),
        "{read}"
    );
    h.ok("bob", &["post", "r", "typed"]);
    let seen = text(&h.worker(&b, &["read", "r", "--peek"]));
    assert!(seen.contains("bob (unverified)"), "{seen}");
}

#[test]
fn a_typed_name_cannot_be_added_on_top_of_a_bundle() {
    let h = Home::new();
    let b = issue(&h, "w-1", "internal", &[]);
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env("KLATCH_HOME", &h.0)
        .env("KLATCH_IDENTITY_FILE", &b)
        .env("KLATCH_NAME", "someone-else")
        .env("HOME", h.0.join("fake-home"))
        .args(["post", "r", "x"])
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(text(&o).contains("cannot act as"), "{}", text(&o));
}

#[test]
fn a_revoked_an_expired_and_a_borrowed_bundle_are_refused() {
    let h = Home::new();
    let b = issue(&h, "w-1", "internal", &[]);
    let other = issue(&h, "w-2", "internal", &[]);
    // a bundle edited to claim another member's name does not pass: the token belongs to the first
    let stolen = h.0.join("stolen.bundle");
    let swapped = std::fs::read_to_string(&b)
        .unwrap()
        .replace("member w-1", "member w-2");
    std::fs::write(&stolen, swapped).unwrap();
    std::fs::set_permissions(&stolen, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    // its store key is right but the token is not w-2's
    let o = h.worker(&stolen, &["post", "r", "x"]);
    assert!(
        !o.status.success() && text(&o).contains("not valid"),
        "{}",
        text(&o)
    );
    // revoked
    assert!(h.launcher(&["worker", "revoke", "w-1"]).status.success());
    let o = h.worker(&b, &["post", "r", "x"]);
    assert!(
        !o.status.success() && text(&o).contains("revoked"),
        "{}",
        text(&o)
    );
    assert!(h.worker(&other, &["post", "r", "ok"]).status.success());
}

#[test]
fn a_credential_expires_on_its_own() {
    let h = Home::new();
    // The shortest lifetime is one second. There is deliberately no "it worked in time" post here: on a loaded runner
    // the process can take longer than that to start, so such a post races the expiry (it failed once on CI). That a
    // fresh credential works is shown by every other test in this file; this one shows that it stops on its own, and
    // the message must name the reason.
    let b = issue(&h, "w-1", "internal", &["--ttl", "1"]);
    std::thread::sleep(std::time::Duration::from_millis(2200));
    let o = h.worker(&b, &["post", "r", "too late"]);
    assert!(
        !o.status.success() && text(&o).contains("expired"),
        "{}",
        text(&o)
    );
}

#[test]
fn a_bundle_other_users_can_read_is_refused() {
    let h = Home::new();
    let b = issue(&h, "w-1", "internal", &[]);
    std::fs::set_permissions(&b, std::os::unix::fs::PermissionsExt::from_mode(0o644)).unwrap();
    let o = h.worker(&b, &["post", "r", "x"]);
    assert!(
        !o.status.success() && text(&o).contains("chmod 600"),
        "{}",
        text(&o)
    );
}

#[test]
fn a_worker_cannot_issue_revoke_or_list_credentials() {
    let h = Home::new();
    let b = issue(&h, "w-1", "internal", &[]);
    for args in [
        vec![
            "worker",
            "issue",
            "w-9",
            "--clearance",
            "internal",
            "--out",
            "/tmp/never-written.bundle",
        ],
        vec!["worker", "revoke", "w-1"],
        vec!["worker", "list"],
    ] {
        let o = h.worker(&b, &args);
        assert!(
            !o.status.success() && text(&o).contains("worker cannot"),
            "{args:?}: {}",
            text(&o)
        );
    }
    assert!(!std::path::Path::new("/tmp/never-written.bundle").exists());
}

#[test]
fn a_worker_at_the_lowest_clearance_works_and_a_hook_with_a_bad_bundle_says_so() {
    let h = Home::new();
    let b = issue(&h, "w-1", "public", &[]);
    let o = h.worker(&b, &["post", "r", "ok at the lowest level"]);
    assert!(o.status.success(), "{}", text(&o));
    // a hook given a broken bundle fails open with the visible line, never a failure of the session
    let bad = h.0.join("bad.bundle");
    std::fs::write(&bad, "garbage").unwrap();
    std::fs::set_permissions(&bad, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    let o = h.worker(&bad, &["hook", "--event", "UserPromptSubmit"]);
    assert!(o.status.success());
    assert!(
        text(&o).contains("could not read the rooms"),
        "{}",
        text(&o)
    );
}

#[test]
fn the_lifetime_and_the_clearance_have_limits() {
    let h = Home::new();
    let out = h.0.join("x.bundle");
    for extra in [["--ttl", "0"], ["--ttl", "99999999"]] {
        let mut args = vec![
            "worker",
            "issue",
            "w-1",
            "--clearance",
            "internal",
            "--out",
            out.to_str().unwrap(),
        ];
        args.extend_from_slice(&extra);
        assert!(!h.launcher(&args).status.success(), "{extra:?}");
    }
    let o = h.launcher(&[
        "worker",
        "issue",
        "w-1",
        "--clearance",
        "secret",
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(
        !o.status.success() && text(&o).contains("agent cap"),
        "{}",
        text(&o)
    );
    assert!(!out.exists(), "a bundle was written for a refused issue");
}

#[test]
fn a_worker_whose_session_ends_loses_its_credential() {
    let h = Home::new();
    let b = issue(&h, "w-1", "internal", &[]);
    assert!(h.worker(&b, &["post", "r", "working"]).status.success());
    let end = h.worker(&b, &["hook", "--event", "SessionEnd"]);
    assert!(end.status.success(), "{}", text(&end));
    let o = h.worker(&b, &["post", "r", "after the end"]);
    assert!(
        !o.status.success() && text(&o).contains("revoked"),
        "{}",
        text(&o)
    );
    // the launcher can see it was revoked, and issue another
    assert!(text(&h.launcher(&["worker", "list"])).contains("revoked"));
    std::fs::remove_file(&b).unwrap(); // a bundle is never written over a file
    let again = issue(&h, "w-1", "internal", &[]);
    assert!(h.worker(&again, &["post", "r", "back"]).status.success());
}

#[test]
fn n_workers_are_issued_in_one_call_each_with_its_own_bundle_and_the_same_parent() {
    let h = Home::new();
    let dir = h.0.join("bundles");
    let o = h.launcher(&[
        "worker",
        "issue",
        "--count",
        "3",
        "--prefix",
        "batch",
        "--clearance",
        "internal",
        "--parent",
        "lead",
        "--out-dir",
        dir.to_str().unwrap(),
    ]);
    assert!(o.status.success(), "{}", text(&o));
    let mut tokens = Vec::new();
    for i in 1..=3 {
        let b = dir.join(format!("batch-{i}.bundle"));
        assert_eq!(std::fs::metadata(&b).unwrap().mode() & 0o777, 0o600);
        let token = std::fs::read_to_string(&b)
            .unwrap()
            .lines()
            .find(|l| l.starts_with("token "))
            .unwrap()
            .to_string();
        tokens.push(token);
        assert!(
            h.worker(&b, &["post", "r", &format!("from {i}")])
                .status
                .success()
        );
    }
    tokens.sort();
    tokens.dedup();
    assert_eq!(tokens.len(), 3, "two workers shared a token");
    let list = text(&h.launcher(&["worker", "list"]));
    assert_eq!(list.matches("parent lead").count(), 3, "{list}");
}

#[test]
fn a_batch_that_fails_part_way_leaves_no_live_credential_and_no_bundle() {
    let h = Home::new();
    let dir = h.0.join("bundles");
    std::fs::create_dir_all(&dir).unwrap();
    // the second bundle cannot be written: a file is already there
    std::fs::write(dir.join("batch-2.bundle"), "in the way").unwrap();
    let o = h.launcher(&[
        "worker",
        "issue",
        "--count",
        "3",
        "--prefix",
        "batch",
        "--clearance",
        "internal",
        "--out-dir",
        dir.to_str().unwrap(),
    ]);
    assert!(!o.status.success());
    let list = text(&h.launcher(&["worker", "list"]));
    assert!(!list.contains("live"), "a credential was left live: {list}");
    assert!(
        !dir.join("batch-1.bundle").exists(),
        "a bundle of the failed batch was left"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("batch-2.bundle")).unwrap(),
        "in the way",
        "a file that was there was changed"
    );
}

#[test]
fn the_batch_form_and_the_single_form_cannot_be_mixed_and_the_count_has_a_limit() {
    let h = Home::new();
    let dir = h.0.join("bundles");
    let d = dir.to_str().unwrap();
    for args in [
        vec![
            "worker",
            "issue",
            "w-1",
            "--count",
            "2",
            "--prefix",
            "b",
            "--clearance",
            "internal",
            "--out-dir",
            d,
        ],
        vec![
            "worker",
            "issue",
            "--count",
            "0",
            "--prefix",
            "b",
            "--clearance",
            "internal",
            "--out-dir",
            d,
        ],
        vec![
            "worker",
            "issue",
            "--count",
            "100000",
            "--prefix",
            "b",
            "--clearance",
            "internal",
            "--out-dir",
            d,
        ],
        vec![
            "worker",
            "issue",
            "--count",
            "2",
            "--clearance",
            "internal",
            "--out-dir",
            d,
        ],
        vec!["worker", "issue", "--clearance", "internal"],
    ] {
        assert!(!h.launcher(&args).status.success(), "{args:?}");
    }
    assert!(text(&h.launcher(&["worker", "list"])).trim().is_empty());
}
