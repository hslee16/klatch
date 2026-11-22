//! Joining a room, posting, reading and history, receipts, and the subscription stream.

use super::Direct;
use crate::domain::Message;
use crate::domain::Posted;
use crate::domain::Result;
use crate::gateway::Batch;
use crate::gateway::DEFAULT_WINDOW;
use crate::gateway::Handoff;
use crate::gateway::History;
use crate::gateway::HistoryRange;
use crate::gateway::SharedTransport;
use crate::gateway::Subscription;
use crate::gateway::shared::forward;
use crate::gateway::shared::{Lend, Own};
use crate::store::Store;

/// Joining a room, posting, reading and history, receipts, and the subscription stream.
pub trait Messages {
    fn join(&self, room: &str) -> Result<()>;
    fn post(&self, room: &str, body: &str) -> Result<Posted> {
        self.post_labeled(room, body, None)
    }
    /// Send a direct message: post to the two-member room shared with `to`. Returns the room and the post.
    fn dm(&self, to: &str, body: &str, label: Option<&str>) -> Result<(String, Posted)>;
    /// Post under a label lower than the room's own (`None` for the room's).
    fn post_labeled(&self, room: &str, body: &str, label: Option<&str>) -> Result<Posted>;
    /// Post a message that carries a handoff: a kind and optionally an opaque payload.
    fn post_handoff(
        &self,
        message: (&str, &str, Option<&str>),
        handoff: &crate::gateway::Handoff,
    ) -> Result<Posted>;
    fn read(&self, room: &str, advance: bool) -> Result<Vec<Message>>;
    /// Look back in a room: everyone's messages in `range`, moving no cursor (#518).
    fn history(&self, room: &str, range: HistoryRange) -> Result<History>;
    /// Per-recipient liveness states for a message the caller sent (#546); no text, no read state.
    fn receipt(&self, room: &str, seq: i64) -> Result<crate::domain::Receipts>;
    /// Start a stream of a room from `from_seq` (0 for the beginning). Poll it with `poll`.
    fn subscribe(&self, room: &str, from_seq: i64) -> Result<Subscription>;
    /// The next batch of a subscription, at most `max` messages.
    fn poll(&self, subscription: &mut Subscription, max: usize) -> Result<Batch>;
    /// Acknowledge messages up to `seq`: advances this member's read cursor.
    fn ack(&self, room: &str, seq: i64) -> Result<()>;
}

impl<S: Store> Messages for Direct<S> {
    fn join(&self, room: &str) -> Result<()> {
        match &self.harness {
            Some(h) => self
                .gateway
                .join_with_harness(&self.principal, room, h.as_ref())
                .map(|_| ()),
            None => self.gateway.join(&self.principal, room),
        }
    }
    fn dm(&self, to: &str, body: &str, label: Option<&str>) -> Result<(String, Posted)> {
        self.gateway.dm(&self.principal, to, body, label)
    }
    fn post_labeled(&self, room: &str, body: &str, label: Option<&str>) -> Result<Posted> {
        self.gateway
            .post_labeled(&self.principal, (room, body, label))
    }
    fn post_handoff(
        &self,
        message: (&str, &str, Option<&str>),
        handoff: &crate::gateway::Handoff,
    ) -> Result<Posted> {
        self.gateway.post_handoff(&self.principal, message, handoff)
    }
    fn read(&self, room: &str, advance: bool) -> Result<Vec<Message>> {
        self.gateway.read(&self.principal, room, advance)
    }
    fn history(&self, room: &str, range: HistoryRange) -> Result<History> {
        self.gateway.history(&self.principal, room, range)
    }
    fn receipt(&self, room: &str, seq: i64) -> Result<crate::domain::Receipts> {
        self.gateway.receipt(&self.principal, room, seq)
    }
    fn subscribe(&self, room: &str, from_seq: i64) -> Result<Subscription> {
        Ok(Subscription::new(room, from_seq, DEFAULT_WINDOW))
    }
    fn poll(&self, subscription: &mut Subscription, max: usize) -> Result<Batch> {
        subscription.poll(&self.gateway, &self.principal, max)
    }
    fn ack(&self, room: &str, seq: i64) -> Result<()> {
        self.gateway.ack(&self.principal, room, seq)
    }
}

impl Messages for SharedTransport {
    forward! {
        join(room: &str) -> Result<()>;
        read(room: &str, advance: bool) -> Result<Vec<crate::domain::Message>>;
        history(room: &str, range: HistoryRange) -> Result<crate::gateway::History>;
        receipt(room: &str, seq: i64) -> Result<crate::domain::Receipts>;
        ack(room: &str, seq: i64) -> Result<()>;
    }

    // The calls that store a message also move the posts count, so a waiter's change token moves.
    fn dm(
        &self,
        to: &str,
        body: &str,
        label: Option<&str>,
    ) -> Result<(String, crate::domain::Posted)> {
        let (to, body, label) = (Own::own(&to), Own::own(&body), Own::own(&label));
        self.posting(move |t| t.dm(Lend::lend(&to), Lend::lend(&body), Lend::lend(&label)))
    }
    fn post_labeled(
        &self,
        room: &str,
        body: &str,
        label: Option<&str>,
    ) -> Result<crate::domain::Posted> {
        let (room, body, label) = (Own::own(&room), Own::own(&body), Own::own(&label));
        self.posting(move |t| {
            t.post_labeled(Lend::lend(&room), Lend::lend(&body), Lend::lend(&label))
        })
    }
    fn post_handoff(
        &self,
        (room, body, label): (&str, &str, Option<&str>),
        handoff: &Handoff,
    ) -> Result<crate::domain::Posted> {
        let (room, body, label) = (Own::own(&room), Own::own(&body), Own::own(&label));
        let (kind, payload) = (Own::own(&handoff.kind), Own::own(&handoff.payload));
        self.posting(move |t| {
            let handoff = Handoff {
                kind: Lend::lend(&kind),
                payload: Lend::lend(&payload),
            };
            t.post_handoff(
                (Lend::lend(&room), Lend::lend(&body), Lend::lend(&label)),
                &handoff,
            )
        })
    }
    fn subscribe(&self, room: &str, from_seq: i64) -> Result<Subscription> {
        let room = Own::own(&room);
        self.call(move |t| t.subscribe(Lend::lend(&room), from_seq))
    }
    /// The subscription lives with the caller and goes to the owner for the length of one poll.
    fn poll(&self, subscription: &mut Subscription, max: usize) -> Result<Batch> {
        let mut moved = std::mem::replace(subscription, Subscription::new("", 0, 0));
        let r = self
            .gateway
            .call((&self.principal, &self.harness), move |t| {
                let batch = t.poll(&mut moved, max);
                (batch, moved)
            });
        match r {
            Ok((batch, back)) => {
                *subscription = back;
                batch
            }
            Err(e) => Err(e),
        }
    }
}
