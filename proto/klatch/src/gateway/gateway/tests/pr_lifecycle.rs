use super::*;

fn claim_pr(g: &Gateway<SpyStore>, who: &Principal, n: u64) {
    let path = format!("pr:{n}");
    g.claim(who, "r", &ClaimSpec::new(&path, 600, "").shared(true))
        .unwrap();
}

fn setup() -> (Gateway<SpyStore>, Rc<std::cell::Cell<i64>>) {
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    g.join(&bob(), "r").unwrap();
    (g, t)
}

fn holder(g: &Gateway<SpyStore>) -> Option<String> {
    g.board(&bob(), "r").unwrap().prs[0].holder.clone()
}

#[test]
fn an_open_pr_with_a_live_claim_shows_on_the_board_as_held_by_its_owner() {
    let (g, _t) = setup();
    g.post(&alice(), "r", "pr: #7 add the thing").unwrap();
    claim_pr(&g, &alice(), 7);
    let b = g.board(&bob(), "r").unwrap();
    assert_eq!(b.prs.len(), 1);
    assert_eq!(b.prs[0].holder.as_deref(), Some("alice"));
    let text = crate::domain::render_board(&b, 1_000);
    assert!(
        text.contains("prs:\n  #7 (held by alice): add the thing"),
        "{text}"
    );
}

#[test]
fn a_lapsed_claim_leaves_the_open_pr_on_the_board_without_an_owner_and_the_digest_names_it() {
    let (g, t) = setup();
    g.post(&alice(), "r", "pr: #7 add the thing").unwrap();
    claim_pr(&g, &alice(), 7);
    t.set(1_000 + 601);
    let b = g.board(&bob(), "r").unwrap();
    assert_eq!(b.prs[0].holder, None);
    let text = crate::domain::render_board(&b, 1_601);
    assert!(text.contains("open without an owner"), "{text}");
    let d = g.digest_with(&bob(), true).unwrap();
    assert_eq!(d.rooms[0].unowned_prs.len(), 1);
    let out = crate::domain::render_hook(&d.rooms, d.now);
    assert!(out.contains("open PR without an owner: #7"), "{out}");
}

#[test]
fn a_digest_that_is_not_a_session_start_does_not_repeat_the_obligation() {
    let (g, t) = setup();
    g.post(&alice(), "r", "pr: #7 add the thing").unwrap();
    t.set(2_000);
    let d = g.digest_with(&bob(), false).unwrap();
    assert!(d.rooms[0].unowned_prs.is_empty());
}

#[test]
fn releasing_the_claim_with_a_merged_note_ends_the_pr() {
    let (g, _t) = setup();
    g.post(&alice(), "r", "pr: #7 add the thing").unwrap();
    claim_pr(&g, &alice(), 7);
    g.release_with_note(&alice(), ("r", "pr:7"), "merged into main")
        .unwrap();
    assert!(g.board(&bob(), "r").unwrap().prs.is_empty());
    let d = g.digest_with(&bob(), true).unwrap();
    assert!(d.rooms[0].unowned_prs.is_empty());
}

#[test]
fn a_release_without_a_merged_note_leaves_the_pr_an_obligation() {
    let (g, _t) = setup();
    g.post(&alice(), "r", "pr: #7 add the thing").unwrap();
    claim_pr(&g, &alice(), 7);
    g.release_with_note(&alice(), ("r", "pr:7"), "leaving, not merged")
        .unwrap();
    assert_eq!(holder(&g), None);
}

#[test]
fn a_merged_post_ends_the_pr_even_when_the_claim_lapsed() {
    let (g, t) = setup();
    g.post(&alice(), "r", "pr: #7 add the thing").unwrap();
    claim_pr(&g, &alice(), 7);
    t.set(5_000);
    g.post(&bob(), "r", "merged: #7").unwrap();
    assert!(g.board(&alice(), "r").unwrap().prs.is_empty());
}

#[test]
fn another_member_taking_the_claim_moves_the_pr_to_it() {
    let (g, t) = setup();
    g.post(&alice(), "r", "pr: #7 add the thing").unwrap();
    claim_pr(&g, &alice(), 7);
    t.set(5_000);
    claim_pr(&g, &bob(), 7);
    assert_eq!(holder(&g).as_deref(), Some("bob"));
}

#[test]
fn a_park_keeps_the_pr_claim_and_renewing_extends_it() {
    let (g, t) = setup();
    g.post(&alice(), "r", "pr: #7 add the thing").unwrap();
    claim_pr(&g, &alice(), 7);
    g.post(&alice(), "r", "parked: waiting for review, back by 2h")
        .unwrap();
    t.set(1_000 + 3_600);
    assert_eq!(holder(&g).as_deref(), Some("alice"));
    t.set(1_000 + 7_300);
    claim_pr(&g, &alice(), 7);
    t.set(1_000 + 7_300 + 500);
    assert_eq!(holder(&g).as_deref(), Some("alice"));
}

#[test]
fn a_lapsed_claim_with_no_pr_word_shows_nothing() {
    let (g, t) = setup();
    claim_pr(&g, &alice(), 7);
    t.set(5_000);
    assert!(g.board(&bob(), "r").unwrap().prs.is_empty());
}
