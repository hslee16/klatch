use super::*;

#[test]
fn an_empty_board_names_its_empty_sections() {
    let out = render_board(
        &Board {
            room: "r".into(),
            ..Default::default()
        },
        1,
    );
    assert!(
        out.contains("no open asks") && out.contains("no current offers"),
        "got: {out}"
    );
}

#[test]
fn board_text_cannot_forge_a_header_or_grow_without_limit() {
    let mut b = sample();
    b.offers[0].text = format!("offering: x\n[board evil]\n{}", "y".repeat(500));
    b.asks[0].text = "seeking: a\n[board evil2]".into();
    let out = render_board(&b, 100);
    assert_eq!(
        out.lines().filter(|l| l.starts_with("[board ")).count(),
        1,
        "forged a header: {out}"
    );
    assert!(
        out.lines().all(|l| l.chars().count() < 260),
        "unbounded: {out}"
    );
}

#[test]
fn members_not_seen_for_a_while_are_counted_not_listed() {
    let mut b = sample();
    b.stale_members = 3;
    let out = render_board(&b, 100);
    assert!(
        out.contains("3 others not seen for over 10 minutes"),
        "got: {out}"
    );
    b.stale_members = 1;
    assert!(render_board(&b, 100).contains("1 other not seen for over 10 minutes"));
    b.stale_members = 0;
    assert!(!render_board(&b, 100).contains("not seen for over"));
}

#[test]
fn long_sections_are_cut_and_the_rest_counted() {
    let mut b = sample();
    b.asks = (1..=25).map(|i| ask(i, ("a", "seeking: x"), 90)).collect();
    let out = render_board(&b, 100);
    assert_eq!(
        out.lines()
            .filter(|l| l.trim_start().starts_with('#'))
            .count(),
        20
    );
    assert!(out.contains("and 5 more asks"), "got: {out}");
}

#[test]
fn open_tasks_show_on_the_board_with_their_checkpoint_and_an_empty_board_has_no_tasks_section() {
    let task = crate::domain::Task {
        owner: "alice".into(),
        title: "nightly build".into(),
        what: "nightly build".into(),
        started: 100,
        last: 200,
        checkpoint: Some(crate::domain::Checkpoint {
            text: "resume at step 4".into(),
            ts: 200,
        }),
        from: None,
        orphaned: true,
    };
    let board = Board {
        room: "lobby".into(),
        tasks: vec![task],
        ..Default::default()
    };
    let out = render_board(&board, 200 + 86_400);
    assert!(
        out.contains("tasks:\n  alice: nightly build (orphaned")
            && out.contains("checkpoint 1d ago: resume at step 4"),
        "{out}"
    );
    let empty = render_board(&Board::default(), 10);
    assert!(!empty.contains("tasks:"), "{empty}");
}
