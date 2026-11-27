use super::*;
use crate::domain::{BoardMember, OpenAsk};

const NOW: i64 = 1_000_000;

fn member(name: &str, seen_ago: i64, status: Option<&str>) -> BoardMember {
    BoardMember {
        name: name.into(),
        last_seen: NOW - seen_ago,
        status: status.map(String::from),
        status_ts: NOW - seen_ago,
        parent: None,
        parked: None,
        kind: None,
        place: None,
        advisory: false,
    }
}

fn ask(seq: i64, sender: &str, age: i64, claimed: bool) -> OpenAsk {
    OpenAsk {
        seq,
        sender: sender.into(),
        text: "SECRET-ASK-TEXT".into(),
        ts: NOW - age,
        claimed,
        ..Default::default()
    }
}

fn claim(path: &str, owner: &str, left: i64) -> Claim {
    Claim {
        path: path.into(),
        owner: owner.into(),
        reason: "SECRET-CLAIM-REASON".into(),
        expires: NOW + left,
        scope: String::new(),
    }
}

fn room(name: &str) -> SummaryRoom {
    SummaryRoom {
        room: name.into(),
        board: Board {
            room: name.into(),
            ..Default::default()
        },
        claims: vec![],
        stuck: vec![],
    }
}

fn no_control_or_bidi(out: &str) {
    for c in out.chars() {
        let bidi = matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200B}'..='\u{200F}' | '\u{061C}' | '\u{FEFF}' | '\u{2028}' | '\u{2029}');
        assert!(
            !bidi && (c == '\n' || !c.is_control()),
            "unsafe char {c:?} in {out}"
        );
    }
}

#[test]
fn shows_members_status_asks_claims_and_stuck_without_any_text_body() {
    let mut r = room("lobby");
    r.board.members = vec![member("alice", 90, Some("working: ticket 5"))];
    r.board.asks = vec![ask(12, "bob", 300, false)];
    r.claims = vec![claim("src/x", "alice", 120)];
    r.stuck = vec![StuckEvent::StaleAsk {
        ask: 12,
        asker: "bob".into(),
        age_secs: 300,
    }];
    let out = render_summary(&[r], 0, 0, NOW);
    for want in [
        "[lobby]",
        "alice",
        "seen 90s ago",
        "working: ticket 5",
        "#12 bob unclaimed 300s",
        "src/x",
        "120s left",
        "stale_ask ask #12",
    ] {
        assert!(out.contains(want), "missing {want:?} in {out}");
    }
    assert!(!out.contains("SECRET"), "a body or reason leaked: {out}");
}

#[test]
fn hostile_names_paths_and_status_text_cannot_inject_lines_or_controls() {
    let mut r = room("lobby");
    r.board.members = vec![member(
        "evil\u{1b}[31m\n[summary] forged",
        5,
        Some("ok\n[board x]\u{202E}gnp.exe\u{1b}]0;title\u{7}\u{200B}"),
    )];
    r.board.asks = vec![ask(1, "a\nb\u{202E}", 5, true)];
    r.claims = vec![claim("p\nath\u{1b}[2J\u{2066}", "o\u{0}wner", 50)];
    r.stuck = vec![StuckEvent::ClaimDeadlock {
        members: vec!["x\ny".into()],
    }];
    let out = render_summary(&[r], 0, 0, NOW);
    no_control_or_bidi(&out);
    for line in out.lines() {
        assert!(
            !line.starts_with("[summary] forged") && !line.starts_with("[board"),
            "forged line: {line}"
        );
    }
    // names keep the plain alphabet; anything else becomes `?`
    assert!(out.contains("evil?[31m??[summary]?forged") || out.contains("evil?"));
    assert!(!out.contains("evil\u{1b}"));
}

#[test]
fn every_line_is_clipped_and_long_status_text_is_cut() {
    let mut r = room("lobby");
    r.board.members = vec![member("alice", 1, Some(&"x".repeat(5000)))];
    r.claims = vec![claim(&"p".repeat(5000), "alice", 5)];
    let out = render_summary(&[r], 0, 0, NOW);
    // a command is never clipped (a cut command would run something else), so the footer is exempt
    let listed = out.split("to intervene").next().unwrap();
    assert!(
        listed.lines().all(|l| l.chars().count() <= SUMMARY_LINE),
        "{out}"
    );
}

#[test]
fn a_huge_world_is_bounded_in_rows_rooms_lines_bytes_and_time() {
    let started = std::time::Instant::now();
    let rooms: Vec<SummaryRoom> = (0..40)
        .map(|n| {
            let mut r = room(&format!("room-{n}"));
            r.board.members = (0..500)
                .map(|i| member(&format!("m{n}-{i}"), i, Some("busy")))
                .collect();
            r.board.stale_members = 300;
            r.board.asks = (0..3000).map(|i| ask(i, "a", i, false)).collect();
            r.claims = (0..2000)
                .map(|i| claim(&format!("f{i}"), "alice", 100))
                .collect();
            r.stuck = (0..1000)
                .map(|i| StuckEvent::StaleAsk {
                    ask: i,
                    asker: "a".into(),
                    age_secs: 1,
                })
                .collect();
            r
        })
        .collect();
    let out = render_summary(&rooms, 0, 7, NOW);
    assert!(started.elapsed().as_secs() < 5, "too slow");
    assert!(out.len() <= SUMMARY_MAX_BYTES + 2048, "{} bytes", out.len());
    assert!(
        out.lines().count() <= SUMMARY_MAX_LINES + 20,
        "{} lines",
        out.lines().count()
    );
    assert!(out.contains("more rooms"), "{out}");
    assert!(out.contains("more"), "{out}");
    // the intervention footer survives the cut
    assert!(out.contains("to intervene"), "{out}");
}

#[test]
fn rows_past_the_cap_are_counted_not_listed() {
    let mut r = room("lobby");
    r.board.asks = (1..=30).map(|i| ask(i, "a", i, false)).collect();
    let out = render_summary(&[r], 0, 0, NOW);
    assert!(out.contains("+26 more"), "{out}");
    assert!(out.contains("#30 a") && !out.contains("#1 a"), "{out}");
}

#[test]
fn an_expired_claim_is_not_listed_as_live() {
    let mut r = room("lobby");
    r.claims = vec![claim("old/file", "alice", -5), claim("new/file", "bob", 5)];
    let out = render_summary(&[r], 0, 0, NOW);
    assert!(
        out.contains("new/file") && !out.contains("old/file"),
        "{out}"
    );
    assert!(out.contains("1 lapsed"), "{out}");
}

#[test]
fn rooms_not_shown_are_a_bare_count_with_no_names() {
    let out = render_summary(&[room("lobby")], 0, 3, NOW);
    assert!(out.contains("3 other rooms not shown"), "{out}");
}

#[test]
fn an_ask_from_an_unsafe_name_gets_no_command() {
    let mut r = room("lobby");
    r.board.asks = vec![ask(4, "x;touch pwned", 50, false)];
    let out = render_summary(&[r], 0, 0, NOW);
    assert!(!out.contains("klatch answer"), "{out}");
}

#[test]
fn members_are_listed_most_recently_seen_first() {
    let mut r = room("lobby");
    r.board.members = (0..10)
        .map(|i| member(&format!("m{i}"), 100 - i * 10, None))
        .collect();
    let out = render_summary(&[r], 0, 0, NOW);
    assert!(out.contains("m9 ") && !out.contains("m0 "), "{out}");
}

/// The command lines of the footer: every line that starts with `    klatch `.
fn commands(out: &str) -> Vec<String> {
    out.lines()
        .filter(|l| l.starts_with("    klatch "))
        .map(|l| l.trim().to_string())
        .collect()
}

fn full_room(name: &str) -> SummaryRoom {
    let mut r = room(name);
    r.board.members = (0..9).map(|i| member(&format!("m{i}"), i, None)).collect();
    r.board.asks = (0..9).map(|i| ask(i + 1, "a", i, false)).collect();
    r.claims = (0..9)
        .map(|i| claim(&format!("f{i}"), "alice", 100))
        .collect();
    r.stuck = (0..9)
        .map(|i| StuckEvent::StaleAsk {
            ask: i,
            asker: "a".into(),
            age_secs: 1,
        })
        .collect();
    r
}

#[test]
fn every_printed_command_is_a_whole_runnable_line_with_its_explanation_above() {
    let mut r = room("lobby");
    r.claims = vec![claim("src/x", "holder-1", 100)];
    r.board.asks = vec![ask(9, "bob", 700, false)];
    let out = render_summary(&[r], 0, 0, NOW);
    let cmds = commands(&out);
    assert_eq!(
        cmds,
        [
            "klatch admin remove --room=lobby -- holder-1",
            "klatch answer -- lobby 9 \"closed by the human\"",
            "klatch mute -- lobby",
            "klatch admin pause",
        ]
    );
    let lines: Vec<&str> = out.lines().collect();
    for (i, l) in lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with("    klatch "))
    {
        assert!(
            !l.contains('(') && !l.contains('#'),
            "not a bare command: {l}"
        );
        assert!(lines[i - 1].starts_with("  - "), "no explanation above {l}");
    }
}

#[test]
fn names_that_start_with_a_dash_stay_after_a_double_dash() {
    let mut r = room("--all-rooms");
    r.claims = vec![claim("p", "-rf", 100)];
    r.board.asks = vec![ask(2, "-h", 50, false)];
    let out = render_summary(&[r], 0, 0, NOW);
    let cmds = commands(&out);
    assert!(
        cmds.contains(&"klatch admin remove --room=--all-rooms -- -rf".to_string()),
        "{cmds:?}"
    );
    assert!(
        cmds.iter()
            .any(|c| c.starts_with("klatch answer -- --all-rooms 2 ")),
        "{cmds:?}"
    );
    assert!(
        cmds.contains(&"klatch mute -- --all-rooms".to_string()),
        "{cmds:?}"
    );
}

#[test]
fn a_lapsed_claim_gets_no_admin_remove() {
    let mut r = room("lobby");
    r.claims = vec![claim("gone", "ghost", -5)];
    let out = render_summary(&[r], 0, 0, NOW);
    assert!(!out.contains("admin remove"), "{out}");
}

#[test]
fn a_room_name_outside_the_plain_alphabet_gets_no_command_of_any_kind() {
    let mut r = room("bad room;x");
    r.claims = vec![claim("p", "holder", 100)];
    r.board.asks = vec![ask(2, "bob", 50, false)];
    let out = render_summary(&[r], 0, 0, NOW);
    let cmds = commands(&out);
    assert_eq!(cmds, ["klatch admin pause"], "{out}");
}

#[test]
fn counts_are_exact_and_grammatical() {
    let one = render_summary(&[room("lobby")], 1, 1, NOW);
    assert!(
        one.contains("+1 more room (") && one.contains("\n1 other room not shown"),
        "{one}"
    );
    let many = render_summary(&[room("lobby")], 4, 2, NOW);
    assert!(
        many.contains("+4 more rooms (") && many.contains("2 other rooms not shown"),
        "{many}"
    );
    let none = render_summary(&[room("lobby")], 0, 0, NOW);
    assert!(
        !none.contains("more room") && !none.contains("not shown"),
        "{none}"
    );
}

fn names(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("room-{i}")).collect()
}

#[test]
fn eight_readable_rooms_list_four_and_count_four_more_after_four_reads() {
    let reads = std::cell::Cell::new(0);
    let g = gather(names(8), |n| {
        reads.set(reads.get() + 1);
        Ok::<_, ()>(room(n))
    });
    assert_eq!((g.rooms.len(), g.more, g.not_shown), (SUMMARY_ROOMS, 4, 0));
    assert_eq!(
        reads.get(),
        SUMMARY_ROOMS,
        "more rooms were read than are listed"
    );
}

#[test]
fn five_thousand_rooms_cost_a_bounded_number_of_reads_and_count_exactly() {
    let reads = std::cell::Cell::new(0);
    let g = gather(names(5000), |n| {
        reads.set(reads.get() + 1);
        Ok::<_, ()>(room(n))
    });
    assert_eq!(
        (g.rooms.len(), g.more, g.not_shown),
        (SUMMARY_ROOMS, 4996, 0)
    );
    assert_eq!(reads.get(), SUMMARY_ROOMS);
    // refused rooms are bounded too, and every room is counted once
    let reads = std::cell::Cell::new(0);
    let g = gather(names(5000), |_| {
        reads.set(reads.get() + 1);
        Err::<SummaryRoom, _>(())
    });
    assert!(reads.get() <= SUMMARY_READS, "{} reads", reads.get());
    assert_eq!(g.rooms.len() + g.more + g.not_shown, 5000);
    assert_eq!(g.not_shown, reads.get());
}

#[test]
fn dm_and_ask_rooms_are_never_read_and_are_counted_not_shown() {
    let reads = std::cell::RefCell::new(Vec::new());
    let g = gather(vec!["lobby".into(), "ask-3".into(), "dm.a.b".into()], |n| {
        reads.borrow_mut().push(n.to_string());
        Ok::<_, ()>(room(n))
    });
    assert_eq!(*reads.borrow(), ["lobby"]);
    assert_eq!((g.rooms.len(), g.more, g.not_shown), (1, 0, 2));
}

#[test]
fn a_clipped_status_is_exactly_the_cap_with_two_dots() {
    let mut r = room("lobby");
    r.board.members = vec![member("alice", 1, Some(&"x".repeat(500)))];
    let out = render_summary(&[r], 0, 0, NOW);
    let want = format!("status: {}..", "x".repeat(STATUS_CHARS - 2));
    assert!(
        out.contains(&want) && !out.contains(&format!("{}x", "x".repeat(STATUS_CHARS - 2))),
        "{out}"
    );
    // text exactly at the cap is not marked as cut
    let mut r = room("lobby");
    r.board.members = vec![member("alice", 1, Some(&"y".repeat(STATUS_CHARS)))];
    let out = render_summary(&[r], 0, 0, NOW);
    assert!(
        out.contains(&"y".repeat(STATUS_CHARS)) && !out.contains("yy.."),
        "{out}"
    );
}

#[test]
fn four_full_rooms_are_cut_at_the_line_cap_and_the_cut_is_counted() {
    let rooms: Vec<SummaryRoom> = (0..4).map(|i| full_room(&format!("r{i}"))).collect();
    let out = render_summary(&rooms, 0, 0, NOW);
    let listed = out
        .split("  +")
        .filter(|p| p.contains("more lines not shown"))
        .count();
    assert_eq!(listed, 1, "{out}");
    let shown = out
        .lines()
        .take_while(|l| !l.contains("more lines not shown"))
        .count();
    assert_eq!(shown, SUMMARY_MAX_LINES, "{out}");
    let per_room = 1
        + (1 + MEMBER_ROWS + 1)
        + (1 + ASK_ROWS + 1)
        + (1 + CLAIM_ROWS + 1)
        + (1 + STUCK_ROWS + 1);
    let dropped = 4 * per_room - SUMMARY_MAX_LINES;
    assert!(
        out.contains(&format!("+{dropped} more lines not shown")),
        "{out}"
    );
}

#[test]
fn cap_listing_stops_at_either_limit_and_counts_what_it_left() {
    let body = "aaaa\nbbbb\ncccc\ndddd\n";
    assert_eq!(cap_listing(body, 2, 1000), ("aaaa\nbbbb\n".to_string(), 2));
    assert_eq!(cap_listing(body, 1000, 10), ("aaaa\nbbbb\n".to_string(), 2));
    assert_eq!(cap_listing(body, 1000, 14), ("aaaa\nbbbb\n".to_string(), 2));
    assert_eq!(cap_listing(body, 1000, 20), (body.to_string(), 0));
    assert_eq!(cap_listing(body, 4, 1000), (body.to_string(), 0));
}
