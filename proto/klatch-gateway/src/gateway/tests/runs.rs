//! Run-scoped namespaces (#122): a launcher creates a run, its workers use the rooms `run.<id>.*`, and ending the run
//! removes those rooms and nothing else. A room of a run that does not exist is refused, so a stray worker cannot
//! recreate a torn-down run.

use super::*;
use klatch_core::Message;

#[test]
fn the_rooms_of_a_run_exist_only_while_the_run_does() {
    let g = open();
    assert_eq!(
        rule_of(g.join(&alice(), "run.a.main").unwrap_err()),
        "run-unknown"
    );
    g.admin_run_create(&admin(), "a").unwrap();
    g.join(&alice(), "run.a.main").unwrap();
    g.post(&alice(), "run.a.main", "working").unwrap();
    g.admin_run_end(&admin(), "a", None).unwrap();
    assert_eq!(
        rule_of(g.post(&alice(), "run.a.main", "late").unwrap_err()),
        "run-unknown"
    );
    assert!(is_refused(g.read(&alice(), "run.a.main", false)));
}

#[test]
fn a_room_that_only_looks_like_a_run_room_is_reserved() {
    let g = open();
    g.admin_run_create(&admin(), "a").unwrap();
    for room in ["run.a", "run.", "run..x", "run.nope.x"] {
        assert!(is_refused(g.join(&alice(), room)), "{room}");
    }
    g.join(&alice(), "runway").unwrap();
    g.join(&alice(), "lobby").unwrap();
}

#[test]
fn ending_a_run_removes_its_rooms_and_leaves_every_other_room_and_run_alone() {
    let g = open();
    g.admin_run_create(&admin(), "a").unwrap();
    g.admin_run_create(&admin(), "ab").unwrap();
    for room in ["run.a.main", "run.a.review", "run.ab.main", "lobby"] {
        g.join(&alice(), room).unwrap();
        g.post(&alice(), room, "hello").unwrap();
        g.claim(&alice(), room, &ClaimSpec::new("src/a.rs", 600, ""))
            .unwrap();
    }
    let gone = g.admin_run_end(&admin(), "a", None).unwrap();
    assert_eq!((gone.rooms, gone.messages), (2, 2));
    assert!(
        g.rooms(&alice())
            .unwrap()
            .iter()
            .all(|r| !r.starts_with("run.a."))
    );
    for room in ["run.ab.main", "lobby"] {
        assert_eq!(g.tail(&alice(), room, (0, 10)).unwrap().len(), 1, "{room}");
        assert_eq!(g.claims(&alice(), room).unwrap().len(), 1, "{room}");
    }
    let audit = g.admin_audit(&admin(), 5).unwrap();
    assert!(
        audit
            .iter()
            .any(|a| a.action == "run-end" && a.target == "a" && a.detail.contains("2 room")),
        "{audit:?}"
    );
}

#[test]
fn the_archive_sees_every_room_and_message_before_anything_is_deleted_and_a_failure_keeps_the_run()
{
    let g = open();
    g.admin_run_create(&admin(), "a").unwrap();
    g.post(&alice(), "run.a.main", "one").unwrap();
    g.post(&bob(), "run.a.main", "two").unwrap();
    g.post(&alice(), "run.a.side", "three").unwrap();
    let mut seen: Vec<(String, Vec<String>)> = Vec::new();
    let mut take = |rooms: &[(String, Vec<Message>)]| -> Result<()> {
        for (room, ms) in rooms {
            seen.push((room.clone(), ms.iter().map(|m| m.body.clone()).collect()));
        }
        Ok(())
    };
    g.admin_run_end(&admin(), "a", Some(&mut take)).unwrap();
    seen.sort();
    assert_eq!(
        seen,
        vec![
            (
                "run.a.main".to_string(),
                vec!["one".to_string(), "two".to_string()]
            ),
            ("run.a.side".to_string(), vec!["three".to_string()]),
        ]
    );
    // an archive that fails stops the teardown: nothing is deleted and the run is still open
    g.admin_run_create(&admin(), "b").unwrap();
    g.post(&alice(), "run.b.main", "keep me").unwrap();
    let mut fail =
        |_: &[(String, Vec<Message>)]| -> Result<()> { Err(Error::Backend("disk full".into())) };
    assert!(g.admin_run_end(&admin(), "b", Some(&mut fail)).is_err());
    assert_eq!(g.tail(&alice(), "run.b.main", (0, 10)).unwrap().len(), 1);
    g.post(&alice(), "run.b.main", "still open").unwrap();
}

#[test]
fn run_ids_are_short_safe_names_and_each_exists_once() {
    let g = open();
    for bad in ["", "has space", "a.b", "x/y", &"r".repeat(33)] {
        assert!(is_invalid(g.admin_run_create(&admin(), bad)), "{bad:?}");
    }
    g.admin_run_create(&admin(), "ok-1_x").unwrap();
    assert!(
        is_invalid(g.admin_run_create(&admin(), "ok-1_x")),
        "a run was created twice"
    );
    assert!(matches!(
        g.admin_run_end(&admin(), "ghost", None),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn the_runs_are_listed_with_their_rooms_and_the_acts_go_through_the_guard() {
    let g = open();
    g.admin_run_create(&admin(), "a").unwrap();
    g.post(&alice(), "run.a.main", "x").unwrap();
    g.post(&alice(), "run.a.side", "y").unwrap();
    let runs = g.admin_runs(&admin()).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!((runs[0].id.as_str(), runs[0].rooms), ("a", 2));
    let denied = Gateway::new(
        SpyStore::new(),
        FixedClock(1),
        Rc::new(SpyGuard::denying(&[""])),
    );
    assert!(is_refused(denied.admin_run_create(&alice(), "a")));
    assert!(is_refused(denied.admin_run_end(&alice(), "a", None)));
    assert!(is_refused(denied.admin_runs(&alice())));
    assert_eq!(denied.store.calls.get(), 0);
}
