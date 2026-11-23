//! A capped `read`, `wait` and wire read show what matters first (#1405): asks addressed to the reader and open asks it
//! could answer, then direct messages, then other messages addressed to it, then the rest in order. The cursor stays
//! exact: what is not shown stays unread, what is shown is never shown again, and `history` stays chronological.
mod common;

use common::*;
use klatch::protocol::ServerFrame;
use serde_json::{Value, json};
use std::collections::BTreeSet;

const MEMBERS: [&str; 5] = ["m1", "m2", "m3", "m4", "m5"];
const PER_MEMBER: usize = 60;
/// Chatter first (seq 1 to 300), then the late items (seq 301 to 305).
const CHATTER: usize = MEMBERS.len() * PER_MEMBER;
const TOTAL: usize = CHATTER + 5;
/// The late items by seq: an ask addressed to alice, plain chatter, an open ask alice could answer, a plain message to
/// alice, more chatter.
const ADDRESSED_ASK: u64 = 301;
const OPEN_ASK: u64 = 303;
const ADDRESSED_PLAIN: u64 = 304;

fn chatter(i: usize) -> String {
    format!("chat {i:04} {}", "x".repeat(390))
}

/// Room "r" with alice joined (cursor 0), 300 messages of about 400 bytes, then five late items (see the constants).
fn mixed_room() -> Home {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    for (k, m) in MEMBERS.iter().enumerate() {
        let lines: Vec<String> = (0..PER_MEMBER)
            .map(|j| {
                let text = chatter(k * PER_MEMBER + j);
                request(
                    &format!("p{j}"),
                    "post",
                    &json!({"room": "r", "body": text}).to_string(),
                )
            })
            .collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        h.serve(m, &refs);
    }
    h.ok(
        "m1",
        &[
            "post",
            "r",
            "seeking: @alice which port does the server use?",
        ],
    );
    h.ok("m2", &["post", "r", &chatter(9001)]);
    h.ok(
        "m3",
        &["post", "r", "seeking: anyone who knows the retry limit"],
    );
    h.ok("m4", &["post", "r", "@alice please rebase before you push"]);
    h.ok("m5", &["post", "r", &chatter(9002)]);
    h
}

/// The seq of every message row, in the order printed: `[SEQ TS SENDER] text` (read) or `[ROOM SEQ TS SENDER] text`
/// (wait). Lines that are notices (`[N more ...`, `[r: N more ...`) are not rows.
fn rows(out: &str) -> Vec<u64> {
    out.lines()
        .filter_map(|l| {
            let inner = l.strip_prefix('[')?;
            let mut words = inner.split(' ');
            let first = words.next()?;
            let n = match first.parse::<u64>() {
                Ok(n) => n,
                Err(_) => words.next()?.parse().ok()?,
            };
            // a row has its time (digits) next; a notice has words
            words.next()?.parse::<u64>().ok()?;
            Some(n)
        })
        .collect()
}

fn unread(h: &Home) -> usize {
    h.ok("alice", &["read", "r", "--count"])
        .trim()
        .parse()
        .unwrap()
}

#[test]
fn the_first_capped_page_starts_with_the_addressed_ask_the_open_ask_and_the_message_to_me() {
    let h = mixed_room();
    let out = h.ok("alice", &["read", "r"]);
    let seqs = rows(&out);
    assert!(out.len() <= 16_500, "{} bytes", out.len());
    assert!(seqs.len() <= 40, "{} messages", seqs.len());
    assert_eq!(
        &seqs[..3],
        &[ADDRESSED_ASK, OPEN_ASK, ADDRESSED_PLAIN],
        "ask to me, open ask I could answer, other message to me, then the rest from the oldest: {seqs:?}"
    );
    assert_eq!(seqs[3], 1, "the rest follows in sequence order: {seqs:?}");
    // the metric: the position of the addressed ask in the first capped page (1 is first). Before the change the page
    // was the oldest unread messages, so the ask was 301st and never on the first page.
    let rank = seqs.iter().position(|s| *s == ADDRESSED_ASK).map(|p| p + 1);
    eprintln!(
        "RANK addressed ask in the first capped read page: {rank:?} of {} shown; the unread order put it at {ADDRESSED_ASK}",
        seqs.len()
    );
    assert_eq!(rank, Some(1));
}

#[test]
fn the_page_says_it_is_in_priority_order_and_how_many_were_left_with_the_exact_command() {
    let h = mixed_room();
    let out = h.ok("alice", &["read", "r"]);
    let shown = rows(&out).len();
    let left = TOTAL - shown;
    assert!(
        out.contains(&format!(
            "[{left} more messages not shown; run `klatch read r` for the next ones]"
        )),
        "{out}"
    );
    assert!(out.contains("[shown by importance"), "{out}");
}

/// Fail if any seq of `page` was already shown; `at` says which call.
fn assert_none_shown_twice(page: &[u64], shown: &[u64], at: &str) {
    for s in page {
        assert!(!shown.contains(s), "seq {s} shown twice{at}");
    }
}

/// Read to the end with `klatch read`, adding what each page shows to `shown`.
fn read_to_the_end(h: &Home, shown: &mut Vec<u64>) {
    loop {
        let page = rows(&h.ok("alice", &["read", "r"]));
        if page.is_empty() {
            return;
        }
        assert_none_shown_twice(&page, shown, "");
        shown.extend(page);
    }
}

#[test]
fn three_reads_in_a_row_show_every_message_once_and_lose_none() {
    let h = mixed_room();
    let mut shown: Vec<u64> = Vec::new();
    for call in 0..3 {
        let page = rows(&h.ok("alice", &["read", "r"]));
        assert!(!page.is_empty(), "call {call}");
        assert_none_shown_twice(&page, &shown, &format!(" (call {call})"));
        shown.extend(&page);
        assert_eq!(
            unread(&h),
            TOTAL - shown.len(),
            "the unread count falls by exactly what was shown (call {call})"
        );
    }
    // the rest, to the end
    read_to_the_end(&h, &mut shown);
    let all: BTreeSet<u64> = shown.iter().copied().collect();
    assert_eq!(shown.len(), TOTAL, "no repeat");
    assert_eq!(
        all,
        (1..=TOTAL as u64).collect::<BTreeSet<_>>(),
        "none lost"
    );
    assert_eq!(unread(&h), 0);
}

#[test]
fn a_message_addressed_to_me_that_arrives_between_calls_is_on_the_next_page() {
    let h = mixed_room();
    let first = rows(&h.ok("alice", &["read", "r"]));
    h.ok("m2", &["post", "r", "seeking: @alice and the timeout?"]);
    let second = rows(&h.ok("alice", &["read", "r"]));
    assert_eq!(second[0], TOTAL as u64 + 1, "{second:?}");
    assert!(first.iter().all(|s| !second.contains(s)), "no repeat");
}

#[test]
fn the_cursor_moves_only_over_a_shown_prefix_and_the_out_of_order_ones_are_remembered() {
    let h = mixed_room();
    let first = rows(&h.ok("alice", &["read", "r"]));
    // 301, 303 and 304 were shown ahead of the older chatter: a plain read cursor would have skipped 1 to 300
    let next = rows(&h.ok("alice", &["read", "r"]));
    assert_eq!(
        next[0],
        first.iter().filter(|s| **s <= CHATTER as u64).count() as u64 + 1
    );
    // 301 is still ahead of the cursor; it must not come back once the cursor reaches it
    let mut seen = BTreeSet::new();
    seen.extend(first);
    seen.extend(next);
    let mut guard = 0;
    loop {
        let page = rows(&h.ok("alice", &["read", "r"]));
        if page.is_empty() {
            break;
        }
        for s in page {
            assert!(seen.insert(s), "seq {s} came back");
        }
        guard += 1;
        assert!(guard < 20);
    }
    assert_eq!(seen.len(), TOTAL);
}

#[test]
fn a_peek_shows_the_same_first_page_marks_nothing_and_continues_chronologically() {
    let h = mixed_room();
    let a = h.ok("alice", &["read", "r", "--peek"]);
    let b = h.ok("alice", &["read", "r", "--peek"]);
    assert_eq!(rows(&a), rows(&b));
    assert_eq!(&rows(&a)[..3], &[ADDRESSED_ASK, OPEN_ASK, ADDRESSED_PLAIN]);
    assert_eq!(unread(&h), TOTAL);
    assert!(a.contains("run `klatch history r --since 0`"), "{a}");
}

#[test]
fn history_stays_strictly_chronological() {
    let h = mixed_room();
    h.ok("alice", &["read", "r"]);
    let out = h.ok("alice", &["history", "r", "--since", "296"]);
    let seqs = rows(&out);
    let mut sorted = seqs.clone();
    sorted.sort_unstable();
    assert_eq!(seqs, sorted, "{out}");
    assert_eq!(seqs.last(), Some(&(TOTAL as u64)));
}

#[test]
fn waits_show_the_same_order_and_three_waits_lose_nothing_and_repeat_nothing() {
    let h = mixed_room();
    let mut shown: Vec<u64> = Vec::new();
    for call in 0..3 {
        let o = h.run("alice", &["wait", "r", "--timeout", "5"]);
        let out = String::from_utf8_lossy(&o.stdout).into_owned();
        let page = rows(&out);
        assert!(!page.is_empty() && page.len() <= 20, "call {call}: {out}");
        if call == 0 {
            assert_eq!(
                &page[..3],
                &[ADDRESSED_ASK, OPEN_ASK, ADDRESSED_PLAIN],
                "{page:?}"
            );
        }
        assert_none_shown_twice(&page, &shown, &format!(" (call {call})"));
        shown.extend(&page);
        assert!(
            out.contains(&format!(
                "[r: {} more messages not shown; run `klatch wait r` for the next ones]",
                TOTAL - shown.len()
            )),
            "call {call}: {out}"
        );
        assert_eq!(unread(&h), TOTAL - shown.len());
    }
    // a read continues, mixing the two commands loses and repeats nothing
    read_to_the_end(&h, &mut shown);
    assert_eq!(
        shown.iter().copied().collect::<BTreeSet<_>>(),
        (1..=TOTAL as u64).collect::<BTreeSet<_>>()
    );
    assert_eq!(shown.len(), TOTAL);
}

#[test]
fn peeking_waits_put_the_addressed_first_and_together_cover_the_room_once() {
    let h = mixed_room();
    let mut shown: Vec<u64> = Vec::new();
    for call in 0..40 {
        let o = h.run("alice", &["wait", "r", "--peek", "--timeout", "0"]);
        let page = rows(&String::from_utf8_lossy(&o.stdout));
        if page.is_empty() {
            break;
        }
        if call == 0 {
            assert_eq!(
                &page[..3],
                &[ADDRESSED_ASK, OPEN_ASK, ADDRESSED_PLAIN],
                "{page:?}"
            );
        }
        assert_none_shown_twice(&page, &shown, &format!(" (call {call})"));
        shown.extend(page);
    }
    assert_eq!(shown.len(), TOTAL);
    assert_eq!(
        shown.iter().copied().collect::<BTreeSet<_>>(),
        (1..=TOTAL as u64).collect::<BTreeSet<_>>()
    );
    // a peek consumed nothing
    assert_eq!(unread(&h), TOTAL);
}

#[test]
fn a_filtered_wait_ranks_its_matches_and_stays_exact() {
    let h = mixed_room();
    let mut shown: Vec<u64> = Vec::new();
    for _ in 0..10 {
        let o = h.run("alice", &["wait", "r:only=seeking:", "--timeout", "0"]);
        let page = rows(&String::from_utf8_lossy(&o.stdout));
        if page.is_empty() {
            break;
        }
        for s in &page {
            assert!(!shown.contains(s), "seq {s} shown twice");
        }
        shown.extend(page);
    }
    assert_eq!(shown, vec![ADDRESSED_ASK, OPEN_ASK]);
}

#[test]
fn a_direct_message_room_is_waited_on_before_a_busier_room_named_first() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    for i in 0..30 {
        h.ok("m1", &["post", "r", &chatter(i)]);
    }
    h.ok("m1", &["dm", "alice", "are you there?"]);
    let out = h.ok("alice", &["wait", "r", "dm:m1", "--timeout", "0"]);
    assert!(out.contains("[dm.alice.m1 "), "{out}");
    assert!(!out.contains("[r "), "{out}");
    // the room named first is not lost: the next wait shows it
    let next = h.ok("alice", &["wait", "r", "dm:m1", "--timeout", "0"]);
    assert!(next.contains("[r "), "{next}");
}

#[test]
fn an_ask_for_me_in_a_room_outranks_a_direct_message_in_another() {
    let h = mixed_room();
    h.ok("m1", &["dm", "alice", "are you there?"]);
    let out = h.ok("alice", &["wait", "dm:m1", "r", "--timeout", "0"]);
    assert!(out.contains("[r 301 "), "{out}");
    assert!(!out.contains("[dm.alice.m1 "), "{out}");
}

fn results(frames: &[ServerFrame]) -> Vec<Value> {
    frames
        .iter()
        .filter_map(|f| match f {
            ServerFrame::Response {
                result: Some(r), ..
            } => Some(r.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_wire_read_with_limits_is_in_priority_order_and_exact_across_calls() {
    let h = mixed_room();
    let params = json!({"room": "r", "max_messages": 20, "max_bytes": 100_000}).to_string();
    let reqs: Vec<String> = (0..16)
        .map(|i| request(&format!("{i}"), "read", &params))
        .collect();
    let refs: Vec<&str> = reqs.iter().map(String::as_str).collect();
    let frames = h.serve("alice", &refs);
    let rs = results(&frames);
    let seq_of = |r: &Value| -> Vec<u64> {
        r["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["seq"].as_u64().unwrap())
            .collect()
    };
    assert_eq!(
        &seq_of(&rs[0])[..3],
        &[ADDRESSED_ASK, OPEN_ASK, ADDRESSED_PLAIN]
    );
    assert_eq!(rs[0]["omitted"], json!(TOTAL - 20));
    let mut shown: Vec<u64> = Vec::new();
    for r in &rs {
        for s in seq_of(r) {
            assert!(!shown.contains(&s), "seq {s} twice");
            shown.push(s);
        }
    }
    assert_eq!(
        shown.iter().copied().collect::<BTreeSet<_>>(),
        (1..=TOTAL as u64).collect::<BTreeSet<_>>()
    );
}

#[test]
fn the_hook_digest_keeps_both_asks_and_the_message_to_me_within_its_cap_and_says_what_it_left() {
    let h = mixed_room();
    let out = h.ok("alice", &["hook", "--event", "UserPromptSubmit"]);
    assert!(out.len() < 9_400, "{} bytes", out.len());
    for must in [
        "which port does the server use",
        "anyone who knows the retry limit",
        "please rebase before you push",
    ] {
        assert!(out.contains(must), "missing {must:?} in {out}");
    }
    assert!(out.contains("more messages not shown"), "{out}");
    assert!(out.contains("klatch read r --peek"), "{out}");
}

#[test]
fn a_hook_digest_and_a_capped_read_do_not_repeat_each_other_and_lose_nothing() {
    let h = mixed_room();
    // a capped read shows the asks first (out of order), the hook after it must not show them again
    let page = rows(&h.ok("alice", &["read", "r"]));
    assert_eq!(&page[..3], &[ADDRESSED_ASK, OPEN_ASK, ADDRESSED_PLAIN]);
    let out = h.ok("alice", &["hook", "--event", "UserPromptSubmit"]);
    assert!(!out.contains("which port does the server use"), "{out}");
    assert!(!out.contains("please rebase before you push"), "{out}");
    // and the read continues without them
    let mut shown: Vec<u64> = page;
    loop {
        let p = rows(&h.ok("alice", &["read", "r"]));
        if p.is_empty() {
            break;
        }
        for s in &p {
            assert!(!shown.contains(s), "seq {s} shown twice");
        }
        shown.extend(p);
    }
    assert_eq!(shown.len(), TOTAL);
}
