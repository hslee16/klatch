//! #188: creating a room and inviting a member are a capability the guard checks and the audit records. The default
//! is open (nothing changes for anyone until the human sets `granted`), so the capability is a floor the human can
//! raise, not a surprise.

use super::*;

fn ada() -> Principal {
    Principal::unattributed("ada")
}

/// Rooms are granted only: `alice` holds the capability, `bob` does not, and `bob` shares room `shared` with alice.
fn granted() -> Gateway<SpyStore> {
    let g = open();
    g.join(&alice(), "shared").unwrap();
    g.join(&bob(), "shared").unwrap();
    g.admin_set_room_creation_granted(&admin(), true).unwrap();
    g.admin_grant_room_capability(&admin(), "alice", true)
        .unwrap();
    g
}

#[test]
fn by_default_any_member_creates_a_room_and_invites() {
    let g = open();
    g.join(&alice(), "new-room").unwrap();
    g.join(&bob(), "shared").unwrap();
    g.join(&alice(), "shared").unwrap();
    g.invite(&alice(), "bob", "new-room", "shared").unwrap();
}

#[test]
fn a_member_without_the_capability_cannot_create_a_room_by_joining_or_posting() {
    let g = granted();
    assert_eq!(
        rule_of(g.join(&bob(), "fresh").unwrap_err()),
        "room-capability"
    );
    assert_eq!(
        rule_of(g.post(&bob(), "fresh2", "hi").unwrap_err()),
        "room-capability"
    );
    // nothing was made: the room has no member and no message
    assert!(g.who(&alice(), "fresh").unwrap().is_empty());
    assert_eq!(g.head(&alice(), "fresh2").unwrap(), 0);
}

#[test]
fn a_member_without_the_capability_still_uses_rooms_that_exist() {
    let g = granted();
    g.join(&alice(), "made").unwrap();
    g.join(&bob(), "made").unwrap();
    g.post(&bob(), "made", "hello").unwrap();
}

#[test]
fn a_member_with_the_capability_creates_a_room() {
    let g = granted();
    g.join(&alice(), "fresh").unwrap();
    g.post(&alice(), "fresh2", "hi").unwrap();
}

#[test]
fn a_member_without_the_capability_cannot_invite_and_nothing_is_posted() {
    let g = granted();
    let before = g.head(&alice(), "shared").unwrap();
    let e = g.invite(&bob(), "alice", "shared", "shared").unwrap_err();
    assert_eq!(rule_of(e), "room-capability");
    assert_eq!(g.head(&alice(), "shared").unwrap(), before);
}

#[test]
fn a_member_with_the_capability_invites_with_a_pointer_in_a_shared_room() {
    let g = granted();
    g.invite(&alice(), "bob", "shared", "shared").unwrap();
    let got = g.read(&bob(), "shared", false).unwrap();
    let text = got.last().unwrap().body.clone();
    assert!(
        text.contains("@bob") && text.contains("klatch join shared"),
        "{text}"
    );
}

#[test]
fn the_audit_names_who_created_the_room_and_who_invited() {
    let g = granted();
    g.join(&alice(), "fresh").unwrap();
    g.invite(&alice(), "bob", "fresh", "shared").unwrap();
    let audit = g.admin_audit(&admin(), 20).unwrap();
    let made = audit.iter().find(|a| a.action == "create-room").unwrap();
    assert_eq!(made.target, "fresh");
    assert!(made.detail.contains("alice"), "{}", made.detail);
    let invited = audit.iter().find(|a| a.action == "invite").unwrap();
    assert_eq!(invited.target, "bob@fresh");
    assert!(invited.detail.contains("alice"), "{}", invited.detail);
}

#[test]
fn a_joining_an_existing_room_is_not_audited_as_a_creation() {
    let g = granted();
    let count = |g: &Gateway<SpyStore>| {
        g.admin_audit(&admin(), 50)
            .unwrap()
            .iter()
            .filter(|a| a.action == "create-room")
            .count()
    };
    let before = count(&g);
    g.join(&bob(), "shared").unwrap();
    assert_eq!(count(&g), before);
}

#[test]
fn a_refusal_is_kept_in_the_refusals_with_the_rule() {
    let g = granted();
    let _ = g.join(&bob(), "fresh");
    let refusals = g.admin_refusals(&admin(), 10).unwrap();
    assert!(
        refusals
            .iter()
            .any(|r| r.member == "bob" && r.rule == "room-capability"),
        "{refusals:?}"
    );
}

#[test]
fn only_the_human_grants_and_an_admin_agent_cannot() {
    let g = granted();
    g.admin_set_admin_agent(&admin(), "ada", true).unwrap();
    assert_eq!(
        rule_of(
            g.admin_grant_room_capability(&ada(), "ada", true)
                .unwrap_err()
        ),
        "admin-scope"
    );
    assert_eq!(
        rule_of(
            g.admin_set_room_creation_granted(&ada(), false)
                .unwrap_err()
        ),
        "admin-scope"
    );
    assert!(!g.can_create_rooms("ada").unwrap());
}

#[test]
fn the_grant_can_be_taken_away_and_the_policy_reopened() {
    let g = granted();
    g.admin_grant_room_capability(&admin(), "alice", false)
        .unwrap();
    assert!(g.join(&alice(), "again").is_err());
    g.admin_set_room_creation_granted(&admin(), false).unwrap();
    g.join(&alice(), "again").unwrap();
}

#[test]
fn the_guard_decides_a_creation_and_an_invite_too() {
    let spy = Rc::new(SpyGuard::allowing());
    let g = Gateway::new(SpyStore::new(), FixedClock(100), spy.clone());
    g.join(&alice(), "shared").unwrap();
    g.join(&bob(), "shared").unwrap();
    g.invite(&alice(), "bob", "shared", "shared").unwrap();
    let seen = spy.seen.borrow().clone();
    let create = Operation::CreateRoom {
        room: "shared".into(),
    };
    assert_eq!(seen.iter().filter(|o| **o == create).count(), 1, "{seen:?}");
    assert!(seen.contains(&Operation::Invite {
        member: "bob".into(),
        room: "shared".into()
    }));
}

// #1195: the grant policy closes room creation, never the two-member room `dm` makes (decisions #631, #672).

#[test]
fn under_granted_a_member_without_the_grant_still_opens_a_dm_and_is_joined_with_its_peer() {
    let g = granted();
    // bob has no grant and the policy is `granted`: a new room is refused, a dm is not
    assert_eq!(
        rule_of(g.join(&bob(), "fresh").unwrap_err()),
        "room-capability"
    );
    let (room, _) = g.dm(&bob(), "carol", "hello", None).unwrap();
    assert_eq!(room, "dm.bob.carol");
    let who: Vec<String> = g
        .who(&bob(), &room)
        .unwrap()
        .into_iter()
        .map(|m| m.name)
        .collect();
    assert!(
        who.contains(&"bob".to_string()) && who.contains(&"carol".to_string()),
        "{who:?}"
    );
    // the peer answers with no grant either
    g.dm(&Principal::unattributed("carol"), "bob", "hi", None)
        .unwrap();
    // and a dm is not audited as a room creation, so the capability never saw it
    assert!(
        !g.admin_audit(&admin(), 50)
            .unwrap()
            .iter()
            .any(|a| a.action == "create-room" && a.target == room)
    );
}

#[test]
fn the_room_refusal_points_a_two_member_need_at_dm() {
    let g = granted();
    let text = match g.join(&bob(), "fresh").unwrap_err() {
        Error::Refused { reason, .. } => reason,
        other => panic!("{other}"),
    };
    assert!(text.contains("klatch dm"), "{text}");
}
