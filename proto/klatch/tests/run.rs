//! Run-scoped namespaces at the command line (#122): `klatch run create|end|list` for a launcher, an encrypted archive
//! written before the rooms go, and no way for a worker to open or end a run.
mod common;
use common::*;
use std::os::unix::fs::MetadataExt;

fn text(o: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

#[test]
fn a_run_is_created_used_listed_and_ended_and_only_its_rooms_go() {
    let h = Home::new();
    assert!(h.launcher(&["run", "create", "a"]).status.success());
    assert!(h.launcher(&["run", "create", "b"]).status.success());
    h.ok("alice", &["post", "run.a.main", "work for run a"]);
    h.ok("alice", &["post", "run.b.main", "work for run b"]);
    h.ok("alice", &["post", "lobby", "an ordinary room"]);
    let list = text(&h.launcher(&["run", "list"]));
    assert!(
        list.contains("a\t") && list.contains("b\t") && list.contains("1 room"),
        "{list}"
    );
    let o = h.launcher(&["run", "end", "a"]);
    assert!(
        o.status.success() && text(&o).contains("1 room"),
        "{}",
        text(&o)
    );
    assert!(
        h.fail("alice", &["post", "run.a.main", "late"])
            .contains("not open")
    );
    assert!(
        h.ok("bob", &["read", "run.b.main", "--peek"])
            .contains("work for run b")
    );
    assert!(
        h.ok("bob", &["read", "lobby", "--peek"])
            .contains("an ordinary room")
    );
}

#[test]
fn a_room_of_a_run_that_was_never_opened_is_refused() {
    let h = Home::new();
    let e = h.fail("alice", &["post", "run.ghost.main", "hello"]);
    assert!(e.contains("not open"), "{e}");
}

#[test]
fn the_archive_is_encrypted_never_overwrites_a_file_and_a_failed_archive_keeps_the_run() {
    let h = Home::new();
    assert!(h.launcher(&["run", "create", "a"]).status.success());
    h.ok("alice", &["post", "run.a.main", "the-archived-secret-text"]);
    let archive = h.0.join("a.archive");
    let o = h.launcher(&["run", "end", "a", "--archive", archive.to_str().unwrap()]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(std::fs::metadata(&archive).unwrap().mode() & 0o777, 0o600);
    let bytes = std::fs::read(&archive).unwrap();
    assert!(
        !bytes.windows(24).any(|w| w == b"the-archived-secret-text"),
        "the archive is readable"
    );
    assert!(bytes.len() > 40);
    // a second run with the same archive path fails and the run stays open with its messages
    assert!(h.launcher(&["run", "create", "b"]).status.success());
    h.ok("alice", &["post", "run.b.main", "must survive"]);
    let o = h.launcher(&["run", "end", "b", "--archive", archive.to_str().unwrap()]);
    assert!(!o.status.success());
    assert!(
        h.ok("bob", &["read", "run.b.main", "--peek"])
            .contains("must survive")
    );
    assert_eq!(
        std::fs::read(&archive).unwrap(),
        bytes,
        "a file was overwritten"
    );
}

#[test]
fn a_worker_cannot_open_end_or_list_runs_and_a_bad_id_is_refused() {
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
    for args in [
        vec!["run", "create", "x"],
        vec!["run", "end", "x"],
        vec!["run", "list"],
    ] {
        let o = h.worker(&bundle, &args);
        assert!(
            !o.status.success() && text(&o).contains("worker cannot"),
            "{args:?}: {}",
            text(&o)
        );
    }
    for bad in ["", "has space", "a.b", "x/y"] {
        assert!(
            !h.launcher(&["run", "create", bad]).status.success(),
            "{bad:?}"
        );
    }
}
