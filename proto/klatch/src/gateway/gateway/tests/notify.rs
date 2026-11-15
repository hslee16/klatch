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
