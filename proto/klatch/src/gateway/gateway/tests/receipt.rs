//! Per-message receipts (#546): what a sender can learn about each recipient without reading anyone's cursor.

use super::*;
use crate::domain::Receipt;

fn carol() -> Principal {
    Principal::unattributed("carol")
}

fn room_of_three() -> (Gateway<SpyStore>, Rc<std::cell::Cell<i64>>) {
    let (g, t) = timed();
    for p in [alice(), bob(), carol()] {
        g.join(&p, "r").unwrap();
    }
    (g, t)
}

fn states(g: &Gateway<SpyStore>, who: &Principal, seq: i64) -> Vec<(String, Receipt)> {
    g.receipt(who, "r", seq).unwrap().rows
}

#[test]
fn a_recipient_active_after_the_post_reads_seen_after_and_a_silent_one_not_seen() {
    let (g, t) = room_of_three();
    t.set(t.get() + 10);
    let seq = g.post(&alice(), "r", "hello").unwrap().seq;
    t.set(t.get() + 30);
    g.digest(&bob()).unwrap();
    t.set(t.get() + 5);
    assert_eq!(
        states(&g, &alice(), seq),
        vec![
            ("bob".to_string(), Receipt::SeenAfter { secs: 30 }),
            ("carol".to_string(), Receipt::NotSeen { secs_before: 10 }),
        ]
    );
}

#[test]
fn a_parked_recipient_reads_parked_with_its_return_time() {
    let (g, t) = room_of_three();
    let now = t.get();
    g.post(&carol(), "r", &format!("parked: build, back by {}", "2h"))
        .unwrap();
    let seq = g.post(&alice(), "r", "hello").unwrap().seq;
    let rows = states(&g, &alice(), seq);
    let carol_row = rows.iter().find(|(n, _)| n == "carol").unwrap();
    assert!(
        matches!(carol_row.1, Receipt::Parked { back_by } if back_by == now + 7_200),
        "{rows:?}"
    );
}

#[test]
fn the_sender_is_not_listed_and_a_member_who_joined_after_is_not_a_recipient() {
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    let seq = g.post(&alice(), "r", "hello").unwrap().seq;
    t.set(t.get() + 5);
    g.join(&bob(), "r").unwrap();
    assert!(states(&g, &alice(), seq).is_empty());
}

#[test]
fn only_the_sender_may_ask() {
    let (g, _t) = room_of_three();
    let seq = g.post(&alice(), "r", "hello").unwrap().seq;
    assert!(is_refused(g.receipt(&bob(), "r", seq)));
}

#[test]
fn an_unknown_seq_is_not_found() {
    let (g, _t) = room_of_three();
    g.post(&alice(), "r", "hello").unwrap();
    assert!(matches!(
        g.receipt(&alice(), "r", 99),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn asking_moves_no_recipient_cursor_and_shows_no_message_text() {
    let (g, _t) = room_of_three();
    let seq = g.post(&alice(), "r", "secret body").unwrap().seq;
    let r = g.receipt(&alice(), "r", seq).unwrap();
    assert!(!format!("{r:?}").contains("secret body"));
    assert_eq!(g.read(&bob(), "r", false).unwrap().len(), 1, "still unread");
    assert_eq!(g.digest(&bob()).unwrap().rooms[0].messages.len(), 1);
}

#[test]
fn a_room_the_guard_refuses_gives_no_receipt() {
    let g = Gateway::new(
        SpyStore::new(),
        FixedClock(1),
        Rc::new(SpyGuard::denying(&["locked"])),
    );
    assert!(is_refused(g.receipt(&alice(), "locked", 1)));
}

fn taken(
    g: &Gateway<SpyStore>,
    who: &Principal,
    seq: i64,
    name: &str,
) -> Option<crate::domain::Taken> {
    let r = g.receipt(who, "r", seq).unwrap();
    r.taken.into_iter().find(|(n, _)| n == name).map(|(_, t)| t)
}

#[test]
fn shown_follows_the_hook_and_read_follows_the_read_cursor() {
    let (g, _t) = room_of_three();
    let seq = g.post(&alice(), "r", "hello").unwrap().seq;
    let none = crate::domain::Taken {
        shown: false,
        read: false,
    };
    assert_eq!(taken(&g, &alice(), seq, "bob"), Some(none));
    g.digest(&bob()).unwrap();
    assert_eq!(
        taken(&g, &alice(), seq, "bob"),
        Some(crate::domain::Taken {
            shown: true,
            read: false
        }),
        "the hook showed it"
    );
    assert_eq!(
        taken(&g, &alice(), seq, "carol"),
        Some(none),
        "carol is untouched"
    );
    g.read(&carol(), "r", true).unwrap();
    assert_eq!(
        taken(&g, &alice(), seq, "carol"),
        Some(crate::domain::Taken {
            shown: false,
            read: true
        })
    );
}

#[test]
fn a_muted_recipient_was_shown_only_a_message_that_mentions_it() {
    let (g, _t) = room_of_three();
    g.set_muted(&bob(), "r", true).unwrap();
    let plain = g.post(&alice(), "r", "hello").unwrap().seq;
    let ping = g.post(&alice(), "r", "hello @bob").unwrap().seq;
    g.digest(&bob()).unwrap();
    assert_eq!(
        taken(&g, &alice(), plain, "bob").map(|t| t.shown),
        Some(false)
    );
    assert_eq!(
        taken(&g, &alice(), ping, "bob").map(|t| t.shown),
        Some(true)
    );
}

#[test]
fn a_post_by_the_recipient_moves_neither_flag() {
    let (g, _t) = room_of_three();
    let seq = g.post(&alice(), "r", "hello").unwrap().seq;
    g.post(&bob(), "r", "unrelated").unwrap();
    let t = taken(&g, &alice(), seq, "bob").unwrap();
    assert!(
        !t.shown && !t.read,
        "a write by the recipient is not a read (#555)"
    );
}

#[test]
fn only_the_sender_gets_the_flags() {
    let (g, _t) = room_of_three();
    let seq = g.post(&alice(), "r", "hello").unwrap().seq;
    g.digest(&bob()).unwrap();
    assert!(is_refused(g.receipt(&bob(), "r", seq)));
    assert!(is_refused(g.receipt(&carol(), "r", seq)));
}
