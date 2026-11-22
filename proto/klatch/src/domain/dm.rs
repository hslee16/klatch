//! Direct messages are two-member rooms (DESIGN D3, ticket #43). Their name is made from the two members, in
//! sorted order, so either side finds the same room.

use crate::domain::valid_name;

/// The prefix of every direct-message room. A room with this prefix that was not made by `dm` is refused.
pub const DM_PREFIX: &str = "dm.";

/// How many messages in a row between the same two members in a shared room make the digest suggest a dm (#1190).
pub const DM_NUDGE_MESSAGES: usize = 4;

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
    let crossing = run
        .iter()
        .skip(DM_NUDGE_MESSAGES - 1)
        .find(|r| r.1 == peer)?;
    shown.contains(&crossing.0).then(|| (peer.to_string(), len))
}

/// The line that names the reply command on a message from a dm peer: the same words in the hook digest, `wait` and
/// `read`. A validated name only, never message text.
pub fn dm_reply_hint(peer: &str) -> String {
    format!("dm from {peer}: reply with klatch dm {peer} \"<text>\"")
}

/// The line that suggests moving a long two-member exchange to a dm: validated names, a count and a room.
pub fn move_to_dm_nudge(peer: &str, n: usize, room: &str) -> String {
    format!(
        "klatch: you and {peer} exchanged {n} messages in {room}; for a longer exchange use: klatch dm {peer} \"<text>\""
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

    #[test]
    fn the_help_lines_are_fixed_templates() {
        assert_eq!(
            dm_reply_hint("bob"),
            "dm from bob: reply with klatch dm bob \"<text>\""
        );
        assert_eq!(
            move_to_dm_nudge("bob", 4, "lobby"),
            "klatch: you and bob exchanged 4 messages in lobby; for a longer exchange use: klatch dm bob \"<text>\""
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
