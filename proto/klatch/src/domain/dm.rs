//! Direct messages are two-member rooms (DESIGN D3, ticket #43). Their name is made from the two members, in
//! sorted order, so either side finds the same room.

use crate::domain::valid_name;

/// The prefix of every direct-message room. A room with this prefix that was not made by `dm` is refused.
pub const DM_PREFIX: &str = "dm.";

/// How many messages in a row between the same two members in a shared room make the digest suggest a dm (#1190).
pub const DM_NUDGE_MESSAGES: usize = 4;

/// How many of a room's newest messages the nudge looks at: a bounded read, never the room's whole history.
pub const DM_NUDGE_WINDOW: usize = 64;

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
