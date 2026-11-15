use super::*;

#[test]
fn a_task_whose_owner_has_gone_stays_on_the_board_as_orphaned_with_its_checkpoint() {
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    g.post(&alice(), "r", "working: nightly build").unwrap();
    g.post(&alice(), "r", "checkpoint: resume at step 4")
        .unwrap();
    let b = g.board(&bob(), "r").unwrap();
    assert_eq!(b.tasks.len(), 1);
    assert!(!b.tasks[0].orphaned, "alice is present");
    t.set(1_000 + 3 * 86_400);
    let b = g.board(&bob(), "r").unwrap();
    assert!(b.tasks[0].orphaned, "alice has been gone for days");
    assert_eq!(
        b.tasks[0].checkpoint.as_ref().unwrap().text,
        "resume at step 4"
    );
}

#[test]
fn a_claim_or_a_park_keeps_a_quiet_owner_from_being_orphaned_and_a_handoff_moves_the_task() {
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    g.post(&alice(), "r", "working: review").unwrap();
    g.claim(&alice(), "r", &ClaimSpec::new("doc.md", 7200, ""))
        .unwrap();
    t.set(1_000 + 3600);
    assert!(!g.board(&bob(), "r").unwrap().tasks[0].orphaned);
    g.post(&alice(), "r", "handoff: review to bob").unwrap();
    let b = g.board(&bob(), "r").unwrap();
    assert_eq!(b.tasks[0].owner, "bob");
    assert_eq!(b.tasks[0].from.as_deref(), Some("alice"));
}
