//! The output of one `read`, `wait` or hook digest is bounded (#1390): whole messages up to a byte and a message cap, a
//! count of what was not shown with the exact command to continue, and cursors that move only over what was shown.
mod common;

use common::*;
use klatch::protocol::ServerFrame;
use serde_json::{Value, json};

const MEMBERS: [&str; 5] = ["m1", "m2", "m3", "m4", "m5"];
/// The posts one member may make in a room in a minute is 100, so a big room is posted by several members.
const PER_MEMBER: usize = 60;
const TOTAL: usize = MEMBERS.len() * PER_MEMBER;

fn body(i: usize) -> String {
    format!("msg {i:04} {}", "x".repeat(390))
}

/// A room "r" that alice has joined (cursor 0) and that holds `TOTAL` messages of about 400 bytes from five members.
fn big_room() -> Home {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    for (k, m) in MEMBERS.iter().enumerate() {
        let lines: Vec<String> = (0..PER_MEMBER)
            .map(|j| {
                let text = body(k * PER_MEMBER + j);
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
    h
}

/// The seq in each message row of `read` output (`[SEQ TS SENDER] text`) or `wait` output (`[ROOM SEQ TS SENDER] text`).
fn row_seqs(out: &str) -> Vec<u64> {
    out.lines()
        .filter_map(|l| {
            let inner = l.strip_prefix('[')?;
            let mut words = inner.split(' ');
            let first = words.next()?;
            let n = first.parse::<u64>().ok().or_else(|| {
                if first == "r" {
                    words.next()?.parse().ok()
                } else {
                    None
                }
            })?;
            l.contains("msg ").then_some(n)
        })
        .collect()
}

#[test]
fn one_read_of_a_big_room_prints_at_most_the_cap_and_says_how_to_continue() {
    let h = big_room();
    let out = h.ok("alice", &["read", "r"]);
    assert!(out.len() <= 16_500, "{} bytes", out.len());
    let seqs = row_seqs(&out);
    assert!(
        !seqs.is_empty() && seqs.len() <= 40,
        "{} messages",
        seqs.len()
    );
    let left = TOTAL - seqs.len();
    assert!(
        out.contains(&format!(
            "[{left} more messages not shown; run `klatch read r` for the next ones]"
        )),
        "{out}"
    );
}

#[test]
fn three_reads_in_a_row_show_the_room_in_order_with_nothing_lost_or_repeated() {
    let h = big_room();
    let mut all: Vec<u64> = Vec::new();
    for call in 0..3 {
        let out = h.ok("alice", &["read", "r"]);
        let seqs = row_seqs(&out);
        assert!(!seqs.is_empty(), "call {call}: {out}");
        let unread: usize = h
            .ok("alice", &["read", "r", "--count"])
            .trim()
            .parse()
            .unwrap();
        all.extend(seqs);
        assert_eq!(
            unread,
            TOTAL - all.len(),
            "unread counter after call {call}"
        );
    }
    assert_eq!(all, (1..=all.len() as u64).collect::<Vec<_>>());
    // and the rest is still there, to the last message
    loop {
        let seqs = row_seqs(&h.ok("alice", &["read", "r"]));
        if seqs.is_empty() {
            break;
        }
        all.extend(seqs);
    }
    assert_eq!(all, (1..=TOTAL as u64).collect::<Vec<_>>());
}

#[test]
fn a_peek_is_capped_too_and_points_at_history_since_the_last_shown() {
    let h = big_room();
    let out = h.ok("alice", &["read", "r", "--peek"]);
    assert!(out.len() <= 16_500, "{} bytes", out.len());
    let seqs = row_seqs(&out);
    let last = seqs.last().unwrap();
    assert!(
        out.contains(&format!("run `klatch history r --since {last}`")),
        "{out}"
    );
    // a peek marks nothing read
    let unread: usize = h
        .ok("alice", &["read", "r", "--count"])
        .trim()
        .parse()
        .unwrap();
    assert_eq!(unread, TOTAL);
}

#[test]
fn one_wait_on_a_big_room_is_capped_and_the_next_waits_continue_in_order() {
    let h = big_room();
    let mut all: Vec<u64> = Vec::new();
    for call in 0..3 {
        let o = h.run("alice", &["wait", "r", "--timeout", "5"]);
        let out = String::from_utf8_lossy(&o.stdout).into_owned();
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(out.len() <= 16_500, "call {call}: {} bytes", out.len());
        assert!(err.contains("data from other agents"), "{err}");
        let seqs = row_seqs(&out);
        assert!(
            !seqs.is_empty() && seqs.len() <= 20,
            "call {call}: {} messages",
            seqs.len()
        );
        all.extend(&seqs);
        let left = TOTAL - all.len();
        assert!(
            out.contains(&format!(
                "[r: {left} more messages not shown; run `klatch wait r` for the next ones]"
            )),
            "call {call}: {out}"
        );
    }
    assert_eq!(all, (1..=all.len() as u64).collect::<Vec<_>>());
    let rest = row_seqs(&h.ok("alice", &["read", "r"]));
    assert_eq!(
        rest.first(),
        Some(&(all.len() as u64 + 1)),
        "a read continues where the waits stopped"
    );
}

#[test]
fn a_peeking_wait_that_is_capped_continues_with_the_same_command() {
    let h = big_room();
    let first = row_seqs(&h.ok("alice", &["wait", "r", "--peek", "--timeout", "5"]));
    let second = row_seqs(&h.ok("alice", &["wait", "r", "--peek", "--timeout", "5"]));
    assert!(!first.is_empty() && !second.is_empty());
    assert_eq!(second[0], first[first.len() - 1] + 1, "no gap, no repeat");
}

#[test]
fn a_message_larger_than_the_cap_is_cut_with_a_marker_and_does_not_block() {
    let h = Home::new();
    h.ok("alice", &["join", "r"]);
    h.ok(
        "bob",
        &["post", "r", &format!("huge msg {}", "y".repeat(30_000))],
    );
    h.ok("bob", &["post", "r", "msg small after"]);
    let out = h.ok("alice", &["read", "r"]);
    assert!(out.len() <= 16_500, "{} bytes", out.len());
    assert!(out.contains("[truncated"), "{}", &out[out.len() - 300..]);
    assert!(
        out.contains("run `klatch history r --since 0`"),
        "{}",
        &out[out.len() - 300..]
    );
    // the huge message was consumed (shown cut), the next one is not lost
    let next = h.ok("alice", &["read", "r"]);
    assert!(next.contains("msg small after"), "{next}");
    assert!(!next.contains("huge"), "{next}");
}

#[test]
fn the_wire_read_takes_optional_limits_and_moves_the_cursor_over_what_it_returned() {
    let h = big_room();
    let params = json!({"room": "r", "max_messages": 7, "max_bytes": 100_000}).to_string();
    let frames = h.serve(
        "alice",
        &[
            &request("1", "read", &params),
            &request("2", "read", &params),
        ],
    );
    let results: Vec<Value> = frames
        .iter()
        .filter_map(|f| match f {
            ServerFrame::Response {
                result: Some(r), ..
            } => Some(r.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["messages"].as_array().unwrap().len(), 7);
    assert_eq!(results[0]["omitted"], json!(TOTAL - 7));
    let seq = |r: &Value, i: usize| r["messages"][i]["seq"].as_u64().unwrap();
    assert_eq!(seq(&results[1], 0), seq(&results[0], 6) + 1);
    // without the limits the answer is what it always was: everything, no `omitted`
    let all = h.serve("alice", &[&request("3", "read", r#"{"room":"r"}"#)]);
    let Some(ServerFrame::Response {
        result: Some(r), ..
    }) = all.last()
    else {
        panic!("{all:?}")
    };
    assert_eq!(r["messages"].as_array().unwrap().len(), TOTAL - 14);
    assert!(r.get("omitted").is_none());
}

#[test]
fn a_hook_digest_of_a_big_room_stays_inside_the_hook_cap_and_counts_what_it_left() {
    let h = big_room();
    let out = h.ok("alice", &["hook", "--event", "UserPromptSubmit"]);
    assert!(out.len() < 9_400, "{} bytes", out.len());
    assert!(out.contains("more messages not shown"), "{out}");
}
