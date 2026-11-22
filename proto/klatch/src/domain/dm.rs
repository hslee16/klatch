//! Direct messages are two-member rooms (DESIGN D3, ticket #43). Their name is made from the two members, in
//! sorted order, so either side finds the same room.

use crate::domain::valid_name;

/// The prefix of every direct-message room. A room with this prefix that was not made by `dm` is refused.
pub const DM_PREFIX: &str = "dm.";

/// How many messages in a row between the same two members in a shared room make the digest suggest a dm (#1190).
pub const DM_NUDGE_MESSAGES: usize = 4;

/// The run lengths at which the digest, `wait` and `read` show the nudge: first at 4, and once more at 6 for a pair
/// that is still exchanging with no dm yet (#1222).
pub const DM_NUDGE_THRESHOLDS: [usize; 2] = [DM_NUDGE_MESSAGES, 6];

/// How many of a room's newest messages the nudge looks at: a bounded read, never the room's whole history.
pub const DM_NUDGE_WINDOW: usize = 64;

/// The newest run of messages sent by only two members, from the senders of a room's newest messages in order: the
/// two names (the last sender first) and the length of the run. `None` when one member sent them all or there are none.
/// Pure, so the digest nudge (#1190) and the mediator proposal (#1191) count an exchange the same way.
pub fn exchange_run<'a>(senders: &[&'a str]) -> Option<(&'a str, &'a str, usize)> {
    let last = *senders.last()?;
    let other = *senders.iter().rev().find(|s| **s != last)?;
    let len = senders
        .iter()
        .rev()
        .take_while(|s| **s == last || **s == other)
        .count();
    Some((last, other, len))
}

/// The long exchange `me` is shown the Nth message of (#1190, #1209): from a room's newest messages as `(seq, sender)`
/// pairs in order, and the `seq`s handed to `me` just now, the peer and the length of the newest run of messages sent
/// by only two members, when `me` is one of them, the run is at least `DM_NUDGE_MESSAGES` long, the peer's name is
/// valid, and `shown` carries the first message of the peer at or past the Nth place of the run (the one that crosses
/// the threshold for `me`, since its own messages are never shown to it). That message is handed over once, so the
/// answer is given once, whichever of the digest, `wait` or `read` hands it over. Pure: whether the room has three
/// members and whether the two have a dm are the caller's to check.
pub fn crossing_exchange(me: &str, rows: &[(i64, &str)], shown: &[i64]) -> Option<(String, usize)> {
    let senders: Vec<&str> = rows.iter().map(|r| r.1).collect();
    let (last, other, len) = exchange_run(&senders)?;
    if len < DM_NUDGE_MESSAGES || (last != me && other != me) {
        return None;
    }
    let peer = if last == me { other } else { last };
    if !valid_name(peer) {
        return None;
    }
    let run = &rows[rows.len() - len..];
    // the newest threshold the run has reached whose crossing message was handed over now: the first nudge at 4, a
    // repeat at 6 for a pair that did not move (#1222)
    DM_NUDGE_THRESHOLDS
        .iter()
        .rev()
        .filter(|t| len >= **t)
        .find_map(|t| {
            let crossing = run.iter().skip(*t - 1).find(|r| r.1 == peer)?;
            shown.contains(&crossing.0).then(|| (peer.to_string(), len))
        })
}

/// The second need-info round of one ask between `me` and a peer (#1223), from a room's newest messages as
/// `(seq, sender, body)` in order and the `seq`s handed to `me` just now: the peer and the number of messages the two
/// sent from the first need-info through the second. The structured words decide, never the prose: the peer's
/// `need-info #N:` and `me`'s `info #N:` for the same ask N, then the peer's second `need-info #N:`, which must be
/// among `shown`. It is the pair's first such second round in the window, so it is handed over once and a third round
/// does not trigger again. Pure, like `crossing_exchange`.
pub fn clarification_round(
    me: &str,
    rows: &[(i64, &str, &str)],
    shown: &[i64],
) -> Option<(String, usize)> {
    use crate::domain::parse_clarification;
    let mut done: Vec<&str> = Vec::new();
    for (i, (seq, peer, body)) in rows.iter().enumerate() {
        if *peer == me || done.contains(peer) {
            continue;
        }
        let Some((true, ask, _)) = parse_clarification(body) else {
            continue;
        };
        // the peer's first need-info for this ask, and my info after it
        let first = rows[..i].iter().position(|r| {
            r.1 == *peer && matches!(parse_clarification(r.2), Some((true, n, _)) if n == ask)
        });
        let Some(first) = first else { continue };
        let second_round = rows[first + 1..i].iter().any(|r| {
            r.1 == me && matches!(parse_clarification(r.2), Some((false, n, _)) if n == ask)
        }) && rows[first + 1..i].iter().all(|r| {
            r.1 != *peer || !matches!(parse_clarification(r.2), Some((true, n, _)) if n == ask)
        });
        if !second_round {
            continue;
        }
        done.push(peer);
        if shown.contains(seq) && valid_name(peer) {
            let n = rows[first..=i]
                .iter()
                .filter(|r| r.1 == me || r.1 == *peer)
                .count();
            return Some((peer.to_string(), n));
        }
    }
    None
}

/// Whether `me` is to be shown the dm nudge for the messages handed to it: a long two-member exchange
/// (`crossing_exchange`) or the second need-info round of one ask (`clarification_round`), from a room's newest
/// messages as `(seq, sender, body)`. One answer, so a handover that carries both prints one line.
pub fn dm_nudge(me: &str, rows: &[(i64, &str, &str)], shown: &[i64]) -> Option<(String, usize)> {
    let pairs: Vec<(i64, &str)> = rows.iter().map(|r| (r.0, r.1)).collect();
    crossing_exchange(me, &pairs, shown).or_else(|| clarification_round(me, rows, shown))
}

/// The line that names the reply command on a message from a dm peer: the same words in the hook digest, `wait` and
/// `read`. A validated name only, never message text.
pub fn dm_reply_hint(peer: &str) -> String {
    format!("dm from {peer}: reply with klatch dm {peer} \"<text>\"")
}

/// The line that suggests moving a long two-member exchange to a dm: validated names, a count and a room.
pub fn move_to_dm_nudge(peer: &str, n: usize, room: &str) -> String {
    format!(
        "klatch: you and {peer} exchanged {n} messages in {room}. Do this now: klatch dm {peer} \"<text>\" (send your next message there, not in {room})"
    )
}

/// The room two members share: `dm.<a>.<b>` with the names sorted, or `dm.` and 16 hex digits of a hash of the
/// pair when that would not fit in a room name.
pub fn dm_room(a: &str, b: &str) -> String {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let readable = format!("{DM_PREFIX}{lo}.{hi}");
    if valid_name(&readable) {
        return readable;
    }
    // FNV-1a over the pair, with a separator that no name contains
    let mut h: u64 = 0xcbf29ce484222325;
    for byte in lo.bytes().chain(std::iter::once(b'\n')).chain(hi.bytes()) {
        h = (h ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    format!("{DM_PREFIX}{h:016x}")
}

/// The other member of the direct-message room `room`, read from its name, for `me`, who is one of its two members
/// (#1192). `None` for any other room, for a room made with the short hashed name, and when the name does not settle
/// it (a member name with dots could split two ways). The room name is made by `dm`, and `dm.` is reserved, so the
/// peer is a valid member name.
pub fn dm_peer_in_room(me: &str, room: &str) -> Option<String> {
    let rest = room.strip_prefix(DM_PREFIX)?;
    let before = rest
        .strip_suffix(&format!(".{me}"))
        .filter(|p| !p.is_empty());
    let after = rest
        .strip_prefix(&format!("{me}."))
        .filter(|p| !p.is_empty());
    let peer = match (before, after) {
        (Some(p), None) | (None, Some(p)) => p,
        _ => return None,
    };
    let sorted = dm_room(me, peer) == room;
    (valid_name(peer) && sorted).then(|| peer.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_run_is_the_newest_stretch_sent_by_only_two_members() {
        assert_eq!(
            exchange_run(&["cy", "ann", "bo", "ann", "bo"]),
            Some(("bo", "ann", 4))
        );
        assert_eq!(
            exchange_run(&["ann", "bo", "cy", "bo"]),
            Some(("bo", "cy", 3))
        );
        assert_eq!(exchange_run(&["ann", "ann", "ann"]), None);
        assert_eq!(exchange_run(&[]), None);
    }

    #[test]
    fn the_nudge_belongs_to_the_message_that_crosses_the_threshold() {
        let rows = [(1, "bo"), (2, "ann"), (3, "bo"), (4, "ann"), (5, "bo")];
        // ann is handed bo's 3 and 5; the 4th place of the run is ann's own, so 5 is the crossing message
        assert_eq!(
            crossing_exchange("ann", &rows, &[1, 3, 5]),
            Some(("bo".to_string(), 5))
        );
        assert_eq!(crossing_exchange("ann", &rows, &[1, 3]), None);
        // bo is handed only ann's: the 4th place is ann's message 4
        assert_eq!(
            crossing_exchange("bo", &rows, &[2, 4]),
            Some(("ann".to_string(), 5))
        );
        assert_eq!(crossing_exchange("cy", &rows, &[1, 2, 3, 4, 5]), None);
        assert_eq!(crossing_exchange("ann", &rows[..3], &[1, 3]), None);
    }

    fn alternating(n: i64) -> Vec<(i64, &'static str)> {
        (1..=n)
            .map(|i| (i, if i % 2 == 1 { "ann" } else { "bo" }))
            .collect()
    }

    #[test]
    fn a_pair_still_exchanging_at_six_crosses_a_second_threshold() {
        let rows = alternating(7);
        // ann is handed bo's 6: the 6th place of the run is bo's, so it is the second crossing
        assert_eq!(
            crossing_exchange("ann", &rows[..6], &[6]),
            Some(("bo".to_string(), 6))
        );
        assert_eq!(
            crossing_exchange("ann", &rows[..5], &[4]),
            Some(("bo".to_string(), 5))
        );
        // a message past the threshold that is not the crossing one does not repeat it
        assert_eq!(crossing_exchange("ann", &rows, &[]), None);
        assert_eq!(crossing_exchange("ann", &alternating(8), &[8]), None);
        // bo is handed ann's 7: the 6th place is bo's own, so ann's 7 is the crossing message for bo
        assert_eq!(
            crossing_exchange("bo", &rows, &[7]),
            Some(("ann".to_string(), 7))
        );
        assert_eq!(crossing_exchange("bo", &rows[..6], &[3]), None);
    }

    fn talk<'a>(turns: &[(&'a str, &'a str)]) -> Vec<(i64, &'a str, &'a str)> {
        turns
            .iter()
            .enumerate()
            .map(|(i, (who, body))| (i as i64 + 1, *who, *body))
            .collect()
    }

    #[test]
    fn the_second_need_info_round_of_one_ask_nudges_the_recipient_once() {
        let rows = talk(&[
            ("helper", "need-info #7: which court?"),
            ("alpha", "info #7: the high court"),
            ("helper", "need-info #7: which year?"),
        ]);
        assert_eq!(
            clarification_round("alpha", &rows, &[3]),
            Some(("helper".to_string(), 3))
        );
        // not before it is handed over, and not to the sender
        assert_eq!(clarification_round("alpha", &rows, &[1]), None);
        assert_eq!(clarification_round("helper", &rows, &[2, 3]), None);
        // the first round alone is not enough
        assert_eq!(clarification_round("alpha", &rows[..1], &[1]), None);
        // a third round is not the trigger again
        let mut more = rows.clone();
        more.push((4, "alpha", "info #7: 2024"));
        more.push((5, "helper", "need-info #7: and the docket?"));
        assert_eq!(clarification_round("alpha", &more, &[5]), None);
        assert_eq!(
            clarification_round("alpha", &more, &[3, 5]),
            Some(("helper".to_string(), 3))
        );
    }

    #[test]
    fn the_clarification_trigger_needs_one_ask_one_pair_and_the_forms() {
        let other_ask = talk(&[
            ("helper", "need-info #7: a?"),
            ("alpha", "info #7: x"),
            ("helper", "need-info #8: b?"),
        ]);
        assert_eq!(clarification_round("alpha", &other_ask, &[3]), None);
        let no_info = talk(&[
            ("helper", "need-info #7: a?"),
            ("helper", "need-info #7: b?"),
        ]);
        assert_eq!(clarification_round("alpha", &no_info, &[2]), None);
        let third = talk(&[
            ("helper", "need-info #7: a?"),
            ("cy", "info #7: x"),
            ("helper", "need-info #7: b?"),
        ]);
        assert_eq!(clarification_round("alpha", &third, &[3]), None);
        let prose = talk(&[
            ("helper", "I could say need-info #7: a?"),
            ("alpha", "info #7: x"),
            ("helper", "need-info #7: b?"),
        ]);
        assert_eq!(clarification_round("alpha", &prose, &[3]), None);
        let rev = talk(&[
            ("alpha", "need-info #7: a?"),
            ("helper", "info #7: x"),
            ("alpha", "need-info #7: b?"),
        ]);
        assert_eq!(
            clarification_round("helper", &rev, &[3]),
            Some(("alpha".to_string(), 3))
        );
    }

    #[test]
    fn dm_nudge_takes_either_rule() {
        let rows = talk(&[
            ("helper", "need-info #7: a?"),
            ("alpha", "info #7: x"),
            ("helper", "need-info #7: b?"),
        ]);
        assert_eq!(
            dm_nudge("alpha", &rows, &[3]),
            Some(("helper".to_string(), 3))
        );
        let long = talk(&[("ann", "a"), ("bo", "b"), ("ann", "c"), ("bo", "d")]);
        assert_eq!(dm_nudge("ann", &long, &[4]), Some(("bo".to_string(), 4)));
        assert_eq!(dm_nudge("ann", &long, &[]), None);
    }

    #[test]
    fn the_nudge_is_an_imperative_with_the_exact_command() {
        let line = move_to_dm_nudge("bob", 4, "lobby");
        assert_eq!(
            line,
            "klatch: you and bob exchanged 4 messages in lobby. Do this now: klatch dm bob \"<text>\" (send your next message there, not in lobby)"
        );
        assert!(!line.contains('\n'));
    }

    #[test]
    fn the_help_lines_are_fixed_templates() {
        assert_eq!(
            dm_reply_hint("bob"),
            "dm from bob: reply with klatch dm bob \"<text>\""
        );
    }

    #[test]
    fn the_peer_is_read_from_the_room_name_for_either_side() {
        assert_eq!(
            dm_peer_in_room("bob", "dm.alice.bob").as_deref(),
            Some("alice")
        );
        assert_eq!(
            dm_peer_in_room("alice", "dm.alice.bob").as_deref(),
            Some("bob")
        );
        assert_eq!(dm_peer_in_room("carol", "dm.alice.bob"), None);
        assert_eq!(dm_peer_in_room("bob", "lobby"), None);
        assert_eq!(dm_peer_in_room("bob", "dm.0123456789abcdef"), None);
        assert_eq!(
            dm_peer_in_room("a", "dm.a.a.a"),
            None,
            "a name that does not settle it"
        );
    }

    #[test]
    fn either_side_gets_the_same_room() {
        assert_eq!(dm_room("alice", "bob"), "dm.alice.bob");
        assert_eq!(dm_room("bob", "alice"), "dm.alice.bob");
    }

    #[test]
    fn long_names_fall_back_to_a_stable_short_room() {
        let (a, b) = ("a".repeat(40), "b".repeat(40));
        let room = dm_room(&a, &b);
        assert!(valid_name(&room) && room.starts_with(DM_PREFIX), "{room}");
        assert_eq!(room, dm_room(&b, &a));
        assert_ne!(room, dm_room(&a, &"c".repeat(40)));
    }

    #[test]
    fn a_pair_is_not_confused_with_a_shifted_pair() {
        let long = "x".repeat(30);
        assert_ne!(
            dm_room(&format!("{long}a"), &format!("b{long}")),
            dm_room(&long, &format!("ab{long}"))
        );
    }
}
