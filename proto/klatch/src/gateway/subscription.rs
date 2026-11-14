//! The state of one room subscription: replay from a sequence number, `caught_up`, live tail, and a
//! flow-control window that only advances when the consumer acknowledges (PROTOCOL.md section 6).

use crate::domain::{Message, Principal, Result};
use crate::gateway::Gateway;
use crate::store::Store;

/// Messages a consumer may have outstanding (sent but not acknowledged) before the stream pauses.
pub const DEFAULT_WINDOW: usize = 256;

/// What one poll produced.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Batch {
    pub messages: Vec<Message>,
    /// `Some(seq)` exactly once, when replay has reached the head of the room.
    pub caught_up: Option<i64>,
}

pub struct Subscription {
    room: String,
    from_seq: i64,
    window: usize,
    /// `None` until the first poll fixes the starting point.
    after: Option<i64>,
    acked: i64,
    caught_up: bool,
}

impl Subscription {
    pub fn new(room: &str, from_seq: i64, window: usize) -> Self {
        Self {
            room: room.into(),
            from_seq,
            window,
            after: None,
            acked: 0,
            caught_up: false,
        }
    }

    pub fn room(&self) -> &str {
        &self.room
    }

    /// The highest `seq` handed out so far, or `None` before the first poll.
    pub fn last_sent(&self) -> Option<i64> {
        self.after
    }

    /// Whether `caught_up` has been announced for this subscription.
    pub fn is_caught_up(&self) -> bool {
        self.caught_up
    }

    /// Record an acknowledgement. Only moves forward and never past what was sent.
    pub fn ack(&mut self, seq: i64) {
        let sent = self.after.unwrap_or(0);
        self.acked = self.acked.max(seq.min(sent));
    }

    /// The next batch, at most `max` messages and never more than the window allows.
    pub fn poll<S: Store>(
        &mut self,
        gateway: &Gateway<S>,
        who: &Principal,
        max: usize,
    ) -> Result<Batch> {
        let after = match self.after {
            Some(a) => a,
            None => {
                // A consumer ahead of the room (it was reset) is clamped to the head, so messages
                // posted later are never skipped.
                let head = gateway.head(who, &self.room)?;
                let start = self.from_seq.clamp(0, head);
                self.after = Some(start);
                self.acked = self.acked.max(start);
                start
            }
        };
        let outstanding = (after - self.acked).max(0) as usize;
        let budget = self.window.saturating_sub(outstanding);
        if budget == 0 {
            let caught_up = self.announce_if_at_head(gateway, who, after)?;
            return Ok(Batch {
                messages: Vec::new(),
                caught_up,
            });
        }
        let limit = budget.min(max).max(1);
        let messages = gateway.tail(who, &self.room, (after, limit))?;
        let after = messages.last().map_or(after, |m| m.seq);
        self.after = Some(after);
        let caught_up = if self.caught_up {
            None
        } else if messages.len() < limit {
            self.caught_up = true;
            Some(after)
        } else {
            self.announce_if_at_head(gateway, who, after)?
        };
        Ok(Batch {
            messages,
            caught_up,
        })
    }

    fn announce_if_at_head<S: Store>(
        &mut self,
        gateway: &Gateway<S>,
        who: &Principal,
        after: i64,
    ) -> Result<Option<i64>> {
        if self.caught_up || gateway.head(who, &self.room)? > after {
            return Ok(None);
        }
        self.caught_up = true;
        Ok(Some(after))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Error;
    use crate::gateway::test_support::{SpyGuard, SpyStore};
    use crate::gateway::{AllowAll, FixedClock};
    use std::rc::Rc;

    fn alice() -> Principal {
        Principal::unattributed("alice")
    }
    fn bob() -> Principal {
        Principal::unattributed("bob")
    }
    fn gw() -> Gateway<SpyStore> {
        Gateway::new(SpyStore::new(), FixedClock(10), AllowAll)
    }
    fn fill(g: &Gateway<SpyStore>, n: usize) {
        // every body is different: an identical repeat is refused (R22)
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        for _ in 0..n {
            let i = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            g.post(&bob(), "r", &format!("m{i}")).unwrap();
        }
    }
    fn seqs(b: &Batch) -> Vec<i64> {
        b.messages.iter().map(|m| m.seq).collect()
    }

    #[test]
    fn replays_everything_after_from_seq_then_reports_caught_up_in_one_poll() {
        let g = gw();
        fill(&g, 3);
        let mut s = Subscription::new("r", 1, 100);
        let b = s.poll(&g, &alice(), 50).unwrap();
        assert_eq!((seqs(&b), b.caught_up), (vec![2, 3], Some(3)));
    }

    #[test]
    fn it_reports_whether_caught_up_has_been_announced() {
        let g = gw();
        fill(&g, 3);
        let mut s = Subscription::new("r", 0, 100);
        assert!(!s.is_caught_up());
        s.poll(&g, &alice(), 2).unwrap();
        assert!(!s.is_caught_up(), "two of three sent");
        s.poll(&g, &alice(), 2).unwrap();
        assert!(s.is_caught_up());
    }

    #[test]
    fn an_empty_room_is_caught_up_at_zero() {
        let g = gw();
        let mut s = Subscription::new("r", 0, 100);
        let b = s.poll(&g, &alice(), 50).unwrap();
        assert_eq!((seqs(&b), b.caught_up), (vec![], Some(0)));
    }

    #[test]
    fn replay_spans_polls_and_caught_up_comes_once_at_the_end() {
        let g = gw();
        fill(&g, 5);
        let mut s = Subscription::new("r", 0, 100);
        let b1 = s.poll(&g, &alice(), 2).unwrap();
        assert_eq!((seqs(&b1), b1.caught_up), (vec![1, 2], None));
        let b2 = s.poll(&g, &alice(), 2).unwrap();
        assert_eq!((seqs(&b2), b2.caught_up), (vec![3, 4], None));
        let b3 = s.poll(&g, &alice(), 2).unwrap();
        assert_eq!((seqs(&b3), b3.caught_up), (vec![5], Some(5)));
        let b4 = s.poll(&g, &alice(), 2).unwrap();
        assert_eq!(
            (seqs(&b4), b4.caught_up),
            (vec![], None),
            "caught_up is announced once"
        );
    }

    #[test]
    fn live_messages_arrive_after_caught_up_in_order() {
        let g = gw();
        fill(&g, 1);
        let mut s = Subscription::new("r", 0, 100);
        s.poll(&g, &alice(), 50).unwrap();
        fill(&g, 2);
        let b = s.poll(&g, &alice(), 50).unwrap();
        assert_eq!((seqs(&b), b.caught_up), (vec![2, 3], None));
    }

    #[test]
    fn a_from_seq_beyond_the_head_is_clamped_so_nothing_is_skipped() {
        let g = gw();
        fill(&g, 2);
        let mut s = Subscription::new("r", 99, 100);
        let b = s.poll(&g, &alice(), 50).unwrap();
        assert_eq!((seqs(&b), b.caught_up), (vec![], Some(2)));
        fill(&g, 1);
        assert_eq!(seqs(&s.poll(&g, &alice(), 50).unwrap()), vec![3]);
    }

    #[test]
    fn the_window_stops_the_flow_until_the_consumer_acknowledges() {
        let g = gw();
        fill(&g, 5);
        let mut s = Subscription::new("r", 0, 2);
        assert_eq!(seqs(&s.poll(&g, &alice(), 50).unwrap()), vec![1, 2]);
        assert_eq!(
            seqs(&s.poll(&g, &alice(), 50).unwrap()),
            Vec::<i64>::new(),
            "window full"
        );
        s.ack(1);
        assert_eq!(seqs(&s.poll(&g, &alice(), 50).unwrap()), vec![3]);
        s.ack(3);
        assert_eq!(seqs(&s.poll(&g, &alice(), 50).unwrap()), vec![4, 5]);
    }

    #[test]
    fn an_acknowledgement_beyond_what_was_sent_is_clamped() {
        let g = gw();
        fill(&g, 6);
        let mut s = Subscription::new("r", 0, 2);
        s.poll(&g, &alice(), 50).unwrap(); // sent 1, 2
        s.ack(100);
        assert_eq!(
            seqs(&s.poll(&g, &alice(), 50).unwrap()),
            vec![3, 4],
            "no more than a window ahead of what was sent"
        );
    }

    #[test]
    fn a_backward_acknowledgement_changes_nothing() {
        let g = gw();
        fill(&g, 6);
        let mut s = Subscription::new("r", 0, 2);
        s.poll(&g, &alice(), 50).unwrap();
        s.ack(2);
        s.poll(&g, &alice(), 50).unwrap(); // sent 3, 4
        s.ack(1);
        assert_eq!(seqs(&s.poll(&g, &alice(), 50).unwrap()), Vec::<i64>::new());
    }

    #[test]
    fn caught_up_is_still_announced_when_the_window_is_full_at_the_head() {
        let g = gw();
        fill(&g, 2);
        let mut s = Subscription::new("r", 0, 2);
        let b = s.poll(&g, &alice(), 50).unwrap();
        assert_eq!((seqs(&b), b.caught_up), (vec![1, 2], Some(2)));
        let mut s = Subscription::new("r", 0, 2);
        let first = s.poll(&g, &alice(), 1).unwrap();
        assert_eq!((seqs(&first), first.caught_up), (vec![1], None));
        let second = s.poll(&g, &alice(), 1).unwrap();
        assert_eq!(
            (seqs(&second), second.caught_up),
            (vec![2], Some(2)),
            "announced as soon as the head is reached, even though the window is now full"
        );
        let third = s.poll(&g, &alice(), 1).unwrap();
        assert_eq!(
            (seqs(&third), third.caught_up),
            (vec![], None),
            "and only once"
        );
    }

    #[test]
    fn resuming_from_the_last_seq_does_not_duplicate() {
        let g = gw();
        fill(&g, 3);
        let mut first = Subscription::new("r", 0, 100);
        let b = first.poll(&g, &alice(), 50).unwrap();
        let last = b.messages.last().unwrap().seq;
        fill(&g, 1);
        let mut resumed = Subscription::new("r", last, 100);
        assert_eq!(seqs(&resumed.poll(&g, &alice(), 50).unwrap()), vec![4]);
    }

    #[test]
    fn the_stream_includes_the_subscribers_own_messages_so_seq_has_no_gaps() {
        let g = gw();
        g.post(&alice(), "r", "mine").unwrap();
        fill(&g, 1);
        let mut s = Subscription::new("r", 0, 100);
        assert_eq!(seqs(&s.poll(&g, &alice(), 50).unwrap()), vec![1, 2]);
    }

    #[test]
    fn a_guard_denial_is_an_error_and_nothing_is_read() {
        let g = Gateway::new(
            SpyStore::new(),
            FixedClock(1),
            Rc::new(SpyGuard::denying(&["r"])),
        );
        let mut s = Subscription::new("r", 0, 100);
        assert!(matches!(
            s.poll(&g, &alice(), 50),
            Err(Error::Refused { .. })
        ));
        assert_eq!(g.store_calls(), 0);
    }

    #[test]
    fn a_store_failure_is_an_error() {
        let g = gw();
        g.set_store_failing(true);
        let mut s = Subscription::new("r", 0, 100);
        assert!(matches!(s.poll(&g, &alice(), 50), Err(Error::Backend(_))));
    }
}
