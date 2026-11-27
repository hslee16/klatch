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
    let out = render_summary(&[r], 0, NOW);
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
    let out = render_summary(&[r], 0, NOW);
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
    let out = render_summary(&[r], 0, NOW);
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
    let out = render_summary(&rooms, 7, NOW);
    assert!(started.elapsed().as_secs() < 5, "too slow");
    assert!(out.len() <= SUMMARY_MAX_BYTES + 2048, "{} bytes", out.len());
    assert!(
        out.lines().count() <= SUMMARY_MAX_LINES + 12,
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
    let out = render_summary(&[r], 0, NOW);
    assert!(out.contains("+26 more"), "{out}");
    assert!(out.contains("#30 a") && !out.contains("#1 a"), "{out}");
}

#[test]
fn an_expired_claim_is_not_listed_as_live() {
    let mut r = room("lobby");
    r.claims = vec![claim("old/file", "alice", -5), claim("new/file", "bob", 5)];
    let out = render_summary(&[r], 0, NOW);
    assert!(
        out.contains("new/file") && !out.contains("old/file"),
        "{out}"
    );
    assert!(out.contains("1 lapsed"), "{out}");
}

#[test]
fn rooms_not_shown_are_a_bare_count_with_no_names() {
    let out = render_summary(&[room("lobby")], 3, NOW);
    assert!(out.contains("3 other rooms not shown"), "{out}");
}

#[test]
fn intervention_lines_are_plain_commands_for_safe_names_only() {
    let mut r = room("lobby");
    r.claims = vec![
        claim("src/x", "holder-1", 100),
        claim("y", "bad name;rm", 100),
    ];
    r.board.asks = vec![ask(9, "bob", 700, false)];
    let out = render_summary(&[r], 0, NOW);
    let tail = out.split("to intervene").nth(1).expect("footer");
    for line in tail.lines().skip(1) {
        assert!(
            line.trim().is_empty() || line.starts_with("  klatch ") || line.starts_with("  #"),
            "line: {line}"
        );
    }
    assert!(
        tail.contains("klatch admin remove holder-1 --room lobby"),
        "{tail}"
    );
    assert!(tail.contains("klatch answer lobby 9 "), "{tail}");
    assert!(tail.contains("klatch admin pause"), "{tail}");
    assert!(
        !tail.contains("rm"),
        "an unsafe name reached a command: {tail}"
    );
}

#[test]
fn an_ask_from_an_unsafe_name_gets_no_command() {
    let mut r = room("lobby");
    r.board.asks = vec![ask(4, "x;touch pwned", 50, false)];
    let out = render_summary(&[r], 0, NOW);
    assert!(!out.contains("klatch answer"), "{out}");
}

#[test]
fn members_are_listed_most_recently_seen_first() {
    let mut r = room("lobby");
    r.board.members = (0..10)
        .map(|i| member(&format!("m{i}"), 100 - i * 10, None))
        .collect();
    let out = render_summary(&[r], 0, NOW);
    assert!(out.contains("m9 ") && !out.contains("m0 "), "{out}");
}

#[test]
fn multibyte_text_cannot_push_the_listing_past_the_byte_cap() {
    let rooms: Vec<SummaryRoom> = (0..4)
        .map(|n| {
            let mut r = room(&format!("r{n}"));
            r.board.members = (0..6)
                .map(|i| member(&format!("m{i}"), 1, Some(&"\u{1F600}".repeat(500))))
                .collect();
            r
        })
        .collect();
    let out = render_summary(&rooms, 0, NOW);
    let listed = out.split("to intervene").next().unwrap();
    assert!(
        listed.len() <= SUMMARY_MAX_BYTES + 200,
        "{} bytes",
        listed.len()
    );
}
