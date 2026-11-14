//! Anchored claims at the command line (#125): `claim --anchor`, and a launcher reporting the work item done, by hand
//! (`anchor done`) or by a directory the factory writes to (`anchor reap`).
mod common;
use common::*;

fn text(o: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

#[test]
fn a_launcher_reporting_a_work_item_done_releases_its_claims() {
    let h = Home::new();
    h.ok("alice", &["claim", "r", "src/a.rs", "--anchor", "wo:7"]);
    h.ok("alice", &["claim", "r", "src/b.rs"]);
    assert!(
        h.fail("bob", &["claim", "r", "src/a.rs"])
            .contains("src/a.rs")
    );
    let o = h.launcher(&["anchor", "done", "wo:7"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("released 1"), "{}", text(&o));
    h.ok("bob", &["claim", "r", "src/a.rs"]);
    let held = h.ok("bob", &["claims", "r"]);
    assert!(
        held.contains("src/b.rs"),
        "an unanchored claim was released: {held}"
    );
}

#[test]
fn a_directory_the_factory_writes_to_drives_the_release_by_polling() {
    let h = Home::new();
    h.ok("alice", &["claim", "r", "src/a.rs", "--anchor", "wo:7"]);
    h.ok(
        "alice",
        &["claim", "r", "src/b.rs", "--anchor", "mission/42"],
    );
    let dir = h.0.join("done");
    std::fs::create_dir_all(&dir).unwrap();
    let o = h.launcher(&["anchor", "reap", "--dir", dir.to_str().unwrap()]);
    assert!(
        o.status.success() && text(&o).contains("released 0"),
        "{}",
        text(&o)
    );
    // the factory finishes one item by writing a file named after it (the slash and colon are encoded)
    std::fs::write(dir.join("mission_2f42"), "").unwrap();
    let o = h.launcher(&["anchor", "reap", "--dir", dir.to_str().unwrap()]);
    assert!(text(&o).contains("released 1"), "{}", text(&o));
    let held = h.ok("bob", &["claims", "r"]);
    assert!(
        held.contains("src/a.rs") && !held.contains("src/b.rs"),
        "{held}"
    );
}

#[test]
fn a_worker_cannot_report_work_done_and_a_bad_anchor_is_refused() {
    let h = Home::new();
    let bundle = h.0.join("w-1.bundle");
    assert!(
        h.launcher(&[
            "worker",
            "issue",
            "w-1",
            "--clearance",
            "internal",
            "--out",
            bundle.to_str().unwrap()
        ])
        .status
        .success()
    );
    h.ok("alice", &["claim", "r", "src/a.rs", "--anchor", "wo:7"]);
    let o = h.worker(&bundle, &["anchor", "done", "wo:7"]);
    assert!(
        !o.status.success() && text(&o).contains("worker cannot"),
        "{}",
        text(&o)
    );
    assert!(
        h.fail("bob", &["claim", "r", "src/a.rs"])
            .contains("src/a.rs"),
        "a worker released a claim"
    );
    let bad = h.launcher(&["anchor", "done", "has space"]);
    assert!(!bad.status.success());
    assert!(
        !h.run("alice", &["claim", "r", "x", "--anchor", "bad anchor"])
            .status
            .success()
    );
}
