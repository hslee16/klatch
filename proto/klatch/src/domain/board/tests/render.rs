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
        machine: None,
        undeclared: false,
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

/// A board whose asks and offers are many and long, so its text is far over a small cap.
fn fat_board() -> Board {
    let mut b = sample();
    let long = format!("seeking: {}", "z".repeat(200));
    b.asks = (1..=20).map(|i| ask(i, ("a", &long), 90)).collect();
    b.offers = (1..=20)
        .map(|i| Offer {
            sender: "lib-1".into(),
            seq: 100 + i,
            text: format!("offering: {}", "y".repeat(200)),
            ts: 50,
        })
        .collect();
    b
}

#[test]
fn a_board_within_the_cap_is_unchanged() {
    let b = sample();
    assert_eq!(
        render_board_within(&b, 100, usize::MAX),
        render_board(&b, 100)
    );
    assert!(!render_board(&b, 100).contains("not shown"));
}

#[test]
fn a_board_over_the_cap_loses_entries_counted_and_keeps_every_header() {
    let b = fat_board();
    let full = render_board_within(&b, 100, usize::MAX);
    let cap = 3000;
    assert!(full.len() > cap, "fixture must exceed the cap");
    let out = render_board_within(&b, 100, cap);
    assert!(out.len() <= cap, "{} bytes: {out}", out.len());
    let headers = |t: &str| -> Vec<String> {
        t.lines()
            .filter(|l| !l.starts_with(' '))
            .map(String::from)
            .collect()
    };
    assert_eq!(headers(&out), headers(&full), "a header was lost: {out}");
    let left_out = full.lines().count() - (out.lines().count() - 1);
    assert!(
        out.ends_with(&format!(
            "  [{left_out} more board entries not shown; the board text is capped at {cap} bytes]\n"
        )),
        "got: {out}"
    );
}

#[test]
fn the_cap_never_removes_a_header_even_when_it_is_tiny() {
    let b = fat_board();
    let out = render_board_within(&b, 100, 10);
    assert!(
        out.contains("asks:") && out.contains("offers:") && out.contains("members:"),
        "{out}"
    );
    assert!(out.contains("more board entries not shown"), "{out}");
}

#[test]
fn the_default_cap_is_the_budget_constant() {
    let mut b = fat_board();
    b.asks = (1..=20)
        .map(|i| ask(i, ("a", &format!("seeking: {}", "z".repeat(240))), 90))
        .collect();
    b.tasks = (0..20)
        .map(|i| crate::domain::Task {
            owner: format!("owner{i}"),
            title: "t".repeat(240),
            what: "t".repeat(240),
            started: 1,
            last: 2,
            checkpoint: None,
            machine: None,
            undeclared: false,
            from: None,
            orphaned: false,
        })
        .collect();
    let out = render_board(&b, 100);
    assert!(
        out.len() <= crate::domain::budget::BOARD_MAX_BYTES,
        "{} bytes",
        out.len()
    );
}
