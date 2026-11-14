use super::*;

fn bodies(d: &Digest) -> Vec<&str> {
    d.rooms
        .iter()
        .flat_map(|r| r.messages.iter().map(|m| m.body.as_str()))
        .collect()
}

fn chatty_room() -> Gateway<SpyStore> {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.post(&bob(), "r", "chatter one").unwrap();
    g.post(&bob(), "r", "@alice please look at this").unwrap();
    g.post(&bob(), "r", "chatter two").unwrap();
    g
}

#[test]
fn a_muted_member_is_shown_only_the_messages_that_mention_it() {
    let g = chatty_room();
    g.set_muted(&alice(), "r", true).unwrap();
    let d = g.digest(&alice()).unwrap();
    assert_eq!(bodies(&d), vec!["@alice please look at this"]);
}

#[test]
fn a_mute_hides_nothing_from_an_explicit_read() {
    let g = chatty_room();
    g.set_muted(&alice(), "r", true).unwrap();
    g.digest(&alice()).unwrap();
    assert_eq!(g.read(&alice(), "r", false).unwrap().len(), 3);
}

#[test]
fn an_unmuted_member_is_shown_everything_again() {
    let g = chatty_room();
    g.set_muted(&alice(), "r", true).unwrap();
    g.digest(&alice()).unwrap();
    g.set_muted(&alice(), "r", false).unwrap();
    g.post(&bob(), "r", "chatter three").unwrap();
    assert_eq!(bodies(&g.digest(&alice()).unwrap()), vec!["chatter three"]);
}

#[test]
fn a_mute_is_per_room_and_per_member() {
    let g = chatty_room();
    g.join(&alice(), "side").unwrap();
    g.post(&bob(), "side", "side chatter").unwrap();
    g.join(&bob(), "r").unwrap();
    g.post(&alice(), "r", "from alice").unwrap();
    g.set_muted(&alice(), "r", true).unwrap();
    let alice_sees = bodies(&g.digest(&alice()).unwrap()).join("|");
    assert!(alice_sees.contains("side chatter"), "{alice_sees}");
    assert!(!alice_sees.contains("chatter one"), "{alice_sees}");
    let bob_sees = bodies(&g.digest(&bob()).unwrap()).join("|");
    assert!(bob_sees.contains("from alice"), "{bob_sees}");
}

#[test]
fn muting_validates_the_room_and_goes_through_the_guard() {
    let guard = Rc::new(SpyGuard::allowing());
    let g = Gateway::new(SpyStore::new(), FixedClock(1), guard.clone());
    assert!(is_invalid(g.set_muted(&alice(), "bad room", true)));
    assert!(guard.seen.borrow().is_empty());
    g.set_muted(&alice(), "r", true).unwrap();
    assert_eq!(
        guard.seen.borrow()[0],
        Operation::Mute {
            room: "r".into(),
            muted: true
        }
    );
}
