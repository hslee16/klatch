//! The machine checkpoint through the gateway (#1408): what it records, where it goes, and what it never records.

use super::*;
use crate::domain::ToolClass;

const SECRET: &str = "SENTINEL-sk-live-4242";

fn body_in(g: &Gateway<SpyStore>, room: &str, who: &str) -> Option<String> {
    g.store
        .machine_checkpoints(room)
        .unwrap()
        .into_iter()
        .find(|m| m.member == who)
        .map(|m| m.body)
}

#[test]
fn a_trail_takes_only_a_class_word_and_a_plain_relative_path() {
    let g = open();
    g.join(&alice(), "lobby").unwrap();
    for bad_class in [SECRET, "Edit", "edit now", ""] {
        assert!(
            is_invalid(g.note_tool(&alice(), bad_class, None)),
            "{bad_class}"
        );
    }
    for bad_path in [
        "/etc/passwd",
        "../x",
        "a/../../x",
        "a\nb",
        &format!("{SECRET}/{}", "x".repeat(200)),
        "",
    ] {
        assert!(
            is_invalid(g.note_tool(&alice(), "edit", Some(bad_path))),
            "{bad_path:?}"
        );
    }
    // a path that rides a read or a run is dropped, not stored
    g.note_tool(&alice(), "run", Some("src/secret-looking.rs"))
        .unwrap();
    g.note_tool(&alice(), "edit", Some("src/a.rs")).unwrap();
    let trail = g.store.trail_of("alice").unwrap();
    assert_eq!(trail.files, vec!["src/a.rs"]);
    assert_eq!(trail.tools, vec!["run", "edit"]);
}

#[test]
fn a_stop_with_nothing_to_say_writes_nothing_and_a_session_end_always_writes() {
    let g = open();
    g.join(&alice(), "lobby").unwrap();
    assert_eq!(g.write_machine_checkpoint(&alice(), None).unwrap(), 0);
    assert!(body_in(&g, "lobby", "alice").is_none());
    assert_eq!(
        g.write_machine_checkpoint(&alice(), Some("logout"))
            .unwrap(),
        1
    );
    assert_eq!(
        body_in(&g, "lobby", "alice").as_deref(),
        Some("session ended (logout)")
    );
}

#[test]
fn it_records_claims_open_asks_waiting_asks_and_the_status_word_and_clears_the_trail_at_the_end() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    g.post(&alice(), "r", "working: parser").unwrap();
    g.claim(&alice(), "r", &ClaimSpec::new("src/p.rs", 600, ""))
        .unwrap();
    g.set_status(&alice(), "r", "blocked: waiting for review")
        .unwrap();
    g.note_tool(&alice(), ToolClass::Edit.word(), Some("src/p.rs"))
        .unwrap();
    g.write_machine_checkpoint(&alice(), None).unwrap();
    let b = body_in(&g, "r", "alice").unwrap();
    assert!(
        b.contains("claims src/p.rs") && b.contains("status blocked"),
        "{b}"
    );
    assert!(
        b.contains("files src/p.rs") && b.contains("tools edit"),
        "{b}"
    );
    // the trail survives a Stop and is cleared by the end of the session
    assert_eq!(g.store.trail_of("alice").unwrap().files, vec!["src/p.rs"]);
    g.write_machine_checkpoint(&alice(), Some("clear")).unwrap();
    assert!(g.store.trail_of("alice").unwrap().files.is_empty());
    let end = body_in(&g, "r", "alice").unwrap();
    assert!(end.starts_with("session ended (clear)"), "{end}");
    assert!(
        end.contains("files src/p.rs"),
        "the final one still names the files: {end}"
    );
}

#[test]
fn a_room_the_member_can_no_longer_read_is_not_recorded() {
    let (g, t) = timed();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.join(&alice(), "vault").unwrap();
    g.join(&alice(), "open").unwrap();
    g.post(&alice(), "vault", "working: secret plan").unwrap();
    g.post(&alice(), "open", "working: public plan").unwrap();
    g.note_tool(&alice(), "edit", Some("src/a.rs")).unwrap();
    // alice's clearance is lowered: the vault is now above her
    g.admin_set_clearance(&admin(), "alice", "public").unwrap();
    t.set(2_000);
    g.write_machine_checkpoint(&alice(), None).unwrap();
    assert!(
        body_in(&g, "vault", "alice").is_none(),
        "nothing for a room she cannot read"
    );
    assert!(body_in(&g, "open", "alice").is_some());
}

#[test]
fn a_room_where_the_member_does_no_work_gets_nothing_unless_it_is_the_default_lobby() {
    let g = open();
    g.join(&alice(), "lobby").unwrap();
    g.join(&alice(), "side").unwrap();
    g.note_tool(&alice(), "edit", Some("src/a.rs")).unwrap();
    g.write_machine_checkpoint(&alice(), None).unwrap();
    assert!(body_in(&g, "lobby", "alice").is_some());
    assert!(
        body_in(&g, "side", "alice").is_none(),
        "work paths are not shown in a room where nothing is worked on"
    );
}

#[test]
fn the_explicit_checkpoint_stays_first_and_the_machine_one_is_beside_it_on_the_task() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    g.post(&alice(), "r", "working: parser").unwrap();
    g.post(&alice(), "r", "checkpoint: resume at the grammar")
        .unwrap();
    g.note_tool(&alice(), "edit", Some("src/a.rs")).unwrap();
    g.write_machine_checkpoint(&alice(), None).unwrap();
    let tasks = g.board(&bob(), "r").unwrap().tasks;
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].checkpoint.as_ref().unwrap().text,
        "resume at the grammar"
    );
    assert!(tasks[0].machine.as_ref().unwrap().text.contains("src/a.rs"));
    // a newer explicit checkpoint does not erase the machine one, and never the other way
    g.post(&alice(), "r", "checkpoint: grammar done").unwrap();
    let tasks = g.board(&bob(), "r").unwrap().tasks;
    assert_eq!(tasks[0].checkpoint.as_ref().unwrap().text, "grammar done");
    assert!(tasks[0].machine.is_some());
}

#[test]
fn a_member_who_said_done_leaves_no_stand_in_task_and_an_old_checkpoint_fades_after_a_week() {
    let (g, t) = timed();
    g.join(&alice(), "lobby").unwrap();
    g.join(&bob(), "lobby").unwrap();
    g.set_status(&alice(), "lobby", "done").unwrap();
    g.write_machine_checkpoint(&alice(), Some("other")).unwrap();
    assert!(g.board(&bob(), "lobby").unwrap().tasks.is_empty());
    let carol = Principal::unattributed("carol");
    g.join(&carol, "lobby").unwrap();
    g.note_tool(&carol, "edit", Some("src/c.rs")).unwrap();
    g.write_machine_checkpoint(&carol, Some("other")).unwrap();
    assert_eq!(g.board(&bob(), "lobby").unwrap().tasks.len(), 1);
    t.set(1_000 + crate::domain::TASK_HOLD_SECS + 1);
    assert!(g.board(&bob(), "lobby").unwrap().tasks.is_empty());
}
