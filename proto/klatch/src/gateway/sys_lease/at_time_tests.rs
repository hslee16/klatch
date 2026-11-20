//! The host the lease probe sees (#725): it is the real one except for the clock, so each of its five methods must
//! pass through to the real host (a stub body would make every holder read as gone or unknown).

use super::*;
use crate::harness::Status;

fn tree(tag: &str) -> (SysHost, crate::store::test_dir::TestDir) {
    let d = crate::store::test_dir::TestDir::new(&format!("attime-{tag}"));
    let host = SysHost {
        registry_dir: d.join("sessions"),
        projects_dir: d.join("projects"),
        task_root: d.join("tmp"),
    };
    (host, d)
}

fn rec(session: &str, cwd: &str) -> Record {
    Record {
        pid: 7,
        session_id: session.into(),
        cwd: cwd.into(),
        name: None,
        proc_start: "x".into(),
        pid_domain: "darwin".into(),
    }
}

#[test]
fn the_clock_is_the_one_given_not_the_real_one() {
    let (inner, _dir) = tree("clock");
    for ms in [0, 1, 1_234_000, u64::MAX] {
        let at = AtTime {
            inner: &inner,
            now_ms: ms,
        };
        assert_eq!(at.now_ms(), ms);
    }
}

#[test]
fn the_process_table_is_the_real_one() {
    let (inner, _dir) = tree("procs");
    let at = AtTime {
        inner: &inner,
        now_ms: 1,
    };
    let rows = at.procs().expect("ps works here");
    let me = std::process::id();
    assert!(rows.iter().any(|r| r.pid == me), "this process is listed");
    assert!(rows.len() > 1, "the whole table, not one row");
    assert!(rows.iter().all(|r| r.pid > 0), "no made-up row");
}

#[test]
fn the_turn_is_read_from_the_registry_file_of_the_pid() {
    let (inner, _dir) = tree("turn");
    std::fs::create_dir_all(&inner.registry_dir).unwrap();
    std::fs::write(
        inner.registry_dir.join("77.json"),
        r#"{"pid":77,"status":"busy","statusUpdatedAt":4242}"#,
    )
    .unwrap();
    let at = AtTime {
        inner: &inner,
        now_ms: 1,
    };
    let t = at.turn(77);
    assert_eq!(t.status, Some(Status::Busy));
    assert_eq!(t.updated_ms, Some(4242));
    assert_eq!(at.turn(78), Turn::default(), "no file, no turn");
}

#[test]
fn the_transcript_and_the_task_output_are_the_files_of_the_session() {
    let (inner, _dir) = tree("files");
    let r = rec("abc-1", "/w/x");
    let at = AtTime {
        inner: &inner,
        now_ms: 1,
    };
    assert_eq!(at.transcript(&r), None);
    assert_eq!(at.task_output(&r), None);
    let transcripts = inner.projects_dir.join("-w-x");
    std::fs::create_dir_all(&transcripts).unwrap();
    std::fs::write(transcripts.join("abc-1.jsonl"), "twelve bytes").unwrap();
    let tasks = inner.task_root.join("-w-x/abc-1/tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(tasks.join("out"), "seven b").unwrap();
    assert_eq!(at.transcript(&r).unwrap().len, 12);
    assert_eq!(at.task_output(&r).unwrap().len, 7);
}
