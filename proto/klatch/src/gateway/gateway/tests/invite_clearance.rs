//! #189: an invite never points a member at a room above that member's clearance. The router relies on it (it never
//! sees a clearance), and it holds for every inviter, so the guard is the only place the rule lives.

use super::*;

/// `vault` is private; alice and carol are cleared to private, bob is not; all three share the room `shared`.
fn vault() -> Gateway<SpyStore> {
    let g = open();
    let carol = Principal::unattributed("carol");
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.admin_set_clearance(&admin(), "carol", "private").unwrap();
    for p in [alice(), bob(), carol] {
        g.join(&p, "shared").unwrap();
    }
    g.join(&alice(), "vault").unwrap();
    g
}

#[test]
fn a_member_below_the_rooms_label_is_not_invited_and_nothing_is_posted() {
    let g = vault();
    let before = g.read(&bob(), "shared", false).unwrap().len();
    let e = g.invite(&alice(), "bob", "vault", "shared").unwrap_err();
    assert_eq!(rule_of(e), "clearance");
    assert_eq!(g.read(&bob(), "shared", false).unwrap().len(), before);
}

#[test]
fn a_member_cleared_for_the_room_is_invited() {
    let g = vault();
    g.invite(&alice(), "carol", "vault", "shared").unwrap();
}
