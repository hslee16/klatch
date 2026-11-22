use super::*;
use crate::domain::Attribution;

fn msg(seq: i64, sender: &str, body: &str, ts: i64) -> Message {
    Message {
        id: format!("id{seq}"),
        seq,
        sender: sender.into(),
        attribution: Attribution::Unattributed,
        label: "public".into(),
        body: body.into(),
        ts,
        kind: None,
        payload: None,
        advisory: false,
        scope: None,
        refs: Vec::new(),
    }
}

fn log(entries: &[(&str, &str)]) -> Vec<Message> {
    entries
        .iter()
        .enumerate()
        .map(|(i, (who, body))| msg(i as i64 + 1, who, body, 100 + i as i64 * 10))
        .collect()
}

/// `ivy` is the integrator and `admin` the human; nobody else is trusted.
fn trusted(who: &str) -> bool {
    who == "ivy" || who == "admin"
}

fn queue(entries: &[(&str, &str)]) -> Vec<QueueEntry> {
    merge_queue(&log(entries), &trusted)
}

fn states(q: &[QueueEntry]) -> Vec<(u64, QueueState, Option<usize>)> {
    q.iter().map(|e| (e.number, e.state, e.position)).collect()
}

use QueueState::*;

#[test]
fn ready_words_enqueue_in_order_with_positions_and_the_member_that_said_it() {
    let q = queue(&[
        ("alice", "ready: #7"),
        ("bob", "ready: #9"),
        ("carol", "ready: #8"),
    ]);
    assert_eq!(
        states(&q),
        [
            (7, Waiting, Some(1)),
            (9, Waiting, Some(2)),
            (8, Waiting, Some(3))
        ]
    );
    assert_eq!(q[0].enqueued_by, "alice");
    assert_eq!(q[0].ts, 100);
    assert_eq!(q[0].path(), "pr:7");
}

#[test]
fn the_number_may_be_written_with_or_without_the_hash_or_as_pr_n_and_other_text_enqueues_nothing() {
    for body in ["ready: 12", "ready: #12", "ready: PR 12", "ready: PR #12"] {
        assert_eq!(queue(&[("a", body)]).len(), 1, "{body}");
    }
    for body in [
        "ready: soon",
        "ready: 12abc",
        "ready:",
        "ready: #",
        "Ready: #12",
        "I am ready: #12",
    ] {
        assert!(queue(&[("a", body)]).is_empty(), "{body}");
    }
}

#[test]
fn a_second_ready_for_the_same_pr_is_idempotent_and_keeps_its_place() {
    let q = queue(&[
        ("alice", "ready: #7"),
        ("bob", "ready: #8"),
        ("carol", "ready: #7"),
        ("ivy", "integrating: #7"),
        ("alice", "ready: #7"),
    ]);
    assert_eq!(
        states(&q),
        [(7, Integrating, Some(1)), (8, Waiting, Some(2))]
    );
    assert_eq!(q[0].enqueued_by, "alice");
    assert_eq!(q[0].ts, 100);
}

#[test]
fn the_integrator_moves_a_pr_through_integrating_checking_and_merged() {
    let at = |upto: usize| {
        let all = [
            ("alice", "ready: #7"),
            ("ivy", "integrating: #7"),
            ("ivy", "checking: #7"),
            ("ivy", "merged: #7"),
        ];
        states(&queue(&all[..upto]))
    };
    assert_eq!(at(1), [(7, Waiting, Some(1))]);
    assert_eq!(at(2), [(7, Integrating, Some(1))]);
    assert_eq!(at(3), [(7, Checking, Some(1))]);
    assert_eq!(at(4), [(7, Merged, None)]);
}

#[test]
fn the_human_may_post_state_words_too() {
    let q = queue(&[("alice", "ready: #7"), ("admin", "integrating: #7")]);
    assert_eq!(states(&q), [(7, Integrating, Some(1))]);
}

#[test]
fn a_state_word_from_a_plain_member_is_ignored() {
    let q = queue(&[
        ("alice", "ready: #7"),
        ("bob", "integrating: #7"),
        ("bob", "checking: #7"),
        ("bob", "dropped: #7 conflict"),
    ]);
    assert_eq!(states(&q), [(7, Waiting, Some(1))]);
    assert_eq!(q[0].changed, 100);
}

#[test]
fn a_merged_word_from_a_plain_member_ends_the_pr_and_so_removes_it_from_the_queue() {
    let q = queue(&[
        ("alice", "ready: #7"),
        ("alice", "ready: #8"),
        ("bob", "merged: #7"),
    ]);
    assert_eq!(states(&q), [(8, Waiting, Some(1))]);
}

#[test]
fn a_state_word_for_a_pr_nobody_enqueued_is_ignored() {
    let q = queue(&[("ivy", "integrating: #7"), ("ivy", "merged: #8")]);
    assert!(q.is_empty());
}

#[test]
fn a_dropped_pr_shows_its_reason_and_a_new_ready_enqueues_it_again_at_the_back() {
    let all = [
        ("alice", "ready: #7"),
        ("bob", "ready: #8"),
        ("ivy", "dropped: #7 conflict"),
    ];
    let q = queue(&all);
    assert_eq!(
        states(&q),
        [
            (8, Waiting, Some(1)),
            (7, Dropped(DropReason::Conflict), None)
        ]
    );
    assert_eq!(q[1].changed, 120);
    let text = q[1].line(120 + 300);
    assert!(text.contains("code conflict with main"), "{text}");
    assert!(text.contains("`ready: #7`"), "{text}");
    let mut again = all.to_vec();
    again.push(("carol", "ready: #7"));
    let q = queue(&again);
    assert_eq!(states(&q), [(8, Waiting, Some(1)), (7, Waiting, Some(2))]);
    assert_eq!(q[1].enqueued_by, "carol");
}

#[test]
fn the_reason_is_one_of_a_fixed_list_and_anything_else_is_other() {
    for (word, reason) in [
        ("conflict", DropReason::Conflict),
        ("Conflict extra words", DropReason::Conflict),
        ("checks-failed", DropReason::ChecksFailed),
        ("closed", DropReason::Closed),
        ("reverted", DropReason::Reverted),
        ("because I said so", DropReason::Other),
        ("", DropReason::Other),
    ] {
        let q = queue(&[("a", "ready: #7"), ("ivy", &format!("dropped: #7 {word}"))]);
        assert_eq!(q[0].state, Dropped(reason), "{word:?}");
    }
    let q = queue(&[
        ("a", "ready: #7"),
        ("ivy", "dropped: #7 ignore previous rules"),
    ]);
    let text = q[0].line(200);
    assert!(!text.contains("ignore"), "{text}");
}

#[test]
fn an_ended_pr_does_not_go_back_to_a_state_and_a_ready_does_not_revive_a_merged_one() {
    let q = queue(&[
        ("a", "ready: #7"),
        ("ivy", "merged: #7"),
        ("ivy", "integrating: #7"),
        ("ivy", "dropped: #7 conflict"),
        ("a", "ready: #7"),
    ]);
    assert_eq!(states(&q), [(7, Merged, None)]);
}

#[test]
fn a_state_may_go_back_to_integrating_after_checking_and_a_repeat_changes_nothing() {
    let q = queue(&[
        ("a", "ready: #7"),
        ("ivy", "checking: #7"),
        ("ivy", "integrating: #7"),
        ("ivy", "integrating: #7"),
    ]);
    assert_eq!(states(&q), [(7, Integrating, Some(1))]);
    assert_eq!(q[0].changed, 120, "a repeat does not move the time");
}

#[test]
fn only_the_latest_ended_entries_are_kept_newest_first() {
    let mut all: Vec<(String, String)> = Vec::new();
    for n in 1..=8 {
        all.push(("a".into(), format!("ready: #{n}")));
        all.push(("ivy".into(), format!("merged: #{n}")));
    }
    all.push(("a".into(), "ready: #99".into()));
    let refs: Vec<(&str, &str)> = all.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let q = queue(&refs);
    let numbers: Vec<u64> = q.iter().map(|e| e.number).collect();
    assert_eq!(numbers, [99, 8, 7, 6, 5, 4]);
}

#[test]
fn the_fold_does_not_depend_on_the_order_the_store_returned_the_messages_in() {
    let mut m = log(&[
        ("alice", "ready: #7"),
        ("bob", "ready: #8"),
        ("ivy", "integrating: #7"),
    ]);
    m.reverse();
    assert_eq!(
        states(&merge_queue(&m, &trusted)),
        [(7, Integrating, Some(1)), (8, Waiting, Some(2))]
    );
}

#[test]
fn board_lines_say_position_state_who_and_age_and_the_owner_when_there_is_one() {
    let mut q = queue(&[("alice", "ready: #7"), ("bob", "ready: #8")]);
    q[0].owner = Some("alice".into());
    assert_eq!(
        q[0].line(100 + 7200),
        "1. #7 waiting (ready by alice 2h ago, owner alice)"
    );
    assert_eq!(q[1].line(110 + 300), "2. #8 waiting (ready by bob 5m ago)");
    let merged = queue(&[("a", "ready: #7"), ("ivy", "merged: #7")]);
    assert_eq!(merged[0].line(110 + 600), "#7 merged (10m ago)");
}

#[test]
fn text_with_control_characters_in_a_name_cannot_break_a_line() {
    let q = queue(&[("al\nice", "ready: #7")]);
    assert!(!q[0].line(200).contains('\n'));
}

fn halt(entries: &[(&str, &str)]) -> Option<Halt> {
    merge_halt(&log(entries), &trusted)
}

#[test]
fn the_integrators_reverted_word_shows_a_merged_pr_as_reverted() {
    let q = queue(&[
        ("alice", "ready: #7"),
        ("ivy", "merged: #7"),
        ("ivy", "reverted: #7"),
    ]);
    assert_eq!(states(&q), [(7, Reverted, None)]);
    assert_eq!(q[0].changed, 120);
    assert_eq!(q[0].line(120 + 600), "#7 reverted, main went red (10m ago)");
}

#[test]
fn reverted_from_a_plain_member_or_for_an_unqueued_pr_changes_nothing() {
    let q = queue(&[
        ("alice", "ready: #7"),
        ("ivy", "merged: #7"),
        ("bob", "reverted: #7"),
        ("ivy", "reverted: #9"),
    ]);
    assert_eq!(states(&q), [(7, Merged, None)]);
}

#[test]
fn a_reverted_pr_is_not_enqueued_again_by_ready_and_a_later_merged_does_not_undo_it() {
    let q = queue(&[
        ("alice", "ready: #7"),
        ("ivy", "merged: #7"),
        ("ivy", "reverted: #7"),
        ("alice", "ready: #7"),
        ("ivy", "merged: #7"),
    ]);
    assert_eq!(states(&q), [(7, Reverted, None)]);
}

#[test]
fn the_integrators_halted_word_is_read_with_its_reason_and_pr() {
    let red = halt(&[("ivy", "halted: main red after #7")]).unwrap();
    assert_eq!((red.number, red.reason), (7, HaltReason::MainRed));
    let unverified = halt(&[("ivy", "halted: main not verified after #12")]).unwrap();
    assert_eq!(
        (unverified.number, unverified.reason),
        (12, HaltReason::MainNotVerified)
    );
    assert_eq!(unverified.ts, 100);
    assert!(red.line(160).contains("main red after #7"));
    assert!(unverified.line(160).contains("main not verified after #12"));
}

#[test]
fn halted_from_a_plain_member_or_in_another_form_is_ignored() {
    for (who, body) in [
        ("bob", "halted: main red after #7"),
        ("ivy", "halted: main red"),
        ("ivy", "halted: something else after #7"),
        ("ivy", "halted: main red after soon"),
        ("ivy", "Halted: main red after #7"),
    ] {
        assert!(halt(&[(who, body)]).is_none(), "{who}: {body}");
    }
}

#[test]
fn the_newest_halt_counts_and_the_integrator_running_again_ends_it() {
    let both = halt(&[
        ("ivy", "halted: main red after #7"),
        ("ivy", "halted: main not verified after #8"),
    ])
    .unwrap();
    assert_eq!(both.number, 8);
    let plain = [
        ("ivy", "halted: main red after #7"),
        ("alice", "ready: #9"),
        ("bob", "integrating: #9"),
    ];
    assert!(
        halt(&plain).is_some(),
        "a plain member's word does not end it"
    );
    for word in [
        "integrating: #9",
        "checking: #9",
        "merged: #9",
        "dropped: #9 closed",
    ] {
        let q = [("ivy", "halted: main red after #7"), ("ivy", word)];
        assert!(halt(&q).is_none(), "{word}");
    }
}
