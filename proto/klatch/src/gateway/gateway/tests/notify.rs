//! The notifier seam (#72): a post that was stored is announced to a `Notifier`, and nothing else is.

use super::*;
use crate::gateway::Notifier;
use std::cell::RefCell;

type Log = Rc<RefCell<Vec<(String, i64, String)>>>;

/// A notifier that remembers what it was told.
struct Fake(Log);

impl Notifier for Fake {
    fn posted(&self, room: &str, seq: i64, sender: &str) {
        self.0.borrow_mut().push((room.into(), seq, sender.into()));
    }
}

fn with_fake() -> (Gateway<SpyStore>, Log) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let g =
        Gateway::new(SpyStore::new(), FixedClock(1), AllowAll).with_notifier(Fake(Rc::clone(&log)));
    (g, log)
}

#[test]
fn a_stored_post_is_announced_once_with_its_room_seq_and_sender() {
    let (g, log) = with_fake();
    let p = g.post(&alice(), "r", "hello").unwrap();
    assert_eq!(
        *log.borrow(),
        vec![("r".to_string(), p.seq, "alice".to_string())]
    );
}

#[test]
fn a_refused_post_is_not_announced() {
    let (g, log) = with_fake();
    assert!(g.post(&alice(), "r", "   ").is_err());
    assert!(log.borrow().is_empty());
}

#[test]
fn a_post_the_store_failed_to_write_is_not_announced() {
    let (g, log) = with_fake();
    g.store.fail.set(true);
    assert!(g.post(&alice(), "r", "hello").is_err());
    assert!(log.borrow().is_empty());
}

#[test]
fn with_no_notifier_a_post_works_as_before() {
    let g = Gateway::new(SpyStore::new(), FixedClock(1), AllowAll);
    assert_eq!(g.post(&alice(), "r", "hello").unwrap().seq, 1);
}

/// A write to a socket or a wake pointer says only that something was tried (RESEARCH-NOTES 47.2: a sender session
/// reported "Sent" for messages the receiver held, refused or let expire). Delivery is what the reader's own
/// cursor or hook shows, and nothing a notifier does may move either (#555).
#[test]
fn a_notifier_that_reports_success_does_not_mark_the_message_delivered() {
    let (g, log) = with_fake();
    g.join(&bob(), "r").unwrap();
    g.post(&alice(), "r", "hello").unwrap();
    assert_eq!(log.borrow().len(), 1, "the notifier was told: an attempt");
    assert_eq!(g.wakeable(&bob(), "r").unwrap(), 1, "still unread for bob");
    let digest = g.digest(&bob()).unwrap();
    assert_eq!(
        digest.rooms[0].messages.len(),
        1,
        "the hook has yet to show it"
    );
}

#[test]
fn a_wake_pointer_is_not_a_shown_mark_or_a_read() {
    let g = open();
    g.join(&bob(), "r").unwrap();
    g.post(&alice(), "r", "hello").unwrap();
    g.wake_mark(&bob(), "r").unwrap();
    let digest = g.digest(&bob()).unwrap();
    assert_eq!(
        digest.rooms[0].messages.len(),
        1,
        "the pointer did not use up the hook's showing"
    );
    assert_eq!(
        g.read(&bob(), "r", false).unwrap().len(),
        1,
        "and the cursor did not move"
    );
}
