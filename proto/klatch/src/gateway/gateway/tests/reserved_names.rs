//! #523: the names the system acts under are reserved. A member's gateway refuses `admin` and `canary` as the
//! caller on every path (join, post, the member card), so a typed name never passes for the human or the
//! tripwire. The launcher's gateways are not built for members and keep acting as `admin`.

use super::*;

fn members_gateway() -> Gateway<SpyStore> {
    Gateway::new(SpyStore::new(), FixedClock(100), AllowAll).for_members()
}

#[test]
fn a_reserved_name_cannot_join_post_or_read_a_card_on_a_members_gateway() {
    let g = members_gateway();
    g.join(&alice(), "r").unwrap();
    for name in ["admin", "canary"] {
        let p = Principal::unattributed(name);
        assert_eq!(
            rule_of(g.join(&p, "r").unwrap_err()),
            "reserved-name",
            "{name}"
        );
        assert_eq!(
            rule_of(g.join(&p, "new").unwrap_err()),
            "reserved-name",
            "{name}"
        );
        assert_eq!(
            rule_of(g.post(&p, "r", "hi").unwrap_err()),
            "reserved-name",
            "{name}"
        );
        assert_eq!(
            rule_of(g.member_card(&p, "r", "alice").unwrap_err()),
            "reserved-name",
            "{name}"
        );
    }
    assert!(g.member_card(&alice(), "r", "admin").unwrap().is_none());
}

#[test]
fn the_human_keeps_acting_as_admin_on_the_launchers_gateway() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.admin_set_admin_agent(&admin(), "ada", true).unwrap();
}

// #674: `all` is the broadcast word (`@all`), so no member may take it
#[test]
fn all_is_reserved_so_no_member_collides_with_the_broadcast_word() {
    let g = members_gateway();
    let p = Principal::unattributed("all");
    assert_eq!(rule_of(g.join(&p, "r").unwrap_err()), "reserved-name");
    assert!(crate::domain::is_reserved_name("all"));
}
