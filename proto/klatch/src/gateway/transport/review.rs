//! Crossings between labelled rooms and the reviewer's queue.

use super::Direct;
use crate::domain::Result;
use crate::gateway::SharedTransport;
use crate::gateway::shared::forward;
use crate::store::Store;

/// Crossings between labelled rooms and the reviewer's queue.
pub trait Review {
    /// The caller's own crossings with their state and a fixed reason (#516); never text or a reviewer's reason.
    fn my_crossings(&self, limit: usize) -> Result<Vec<crate::domain::CrossingStatus>>;
    /// The reviewer's two calls (#509): the oldest waiting item it may read, and a verdict on a waiting item.
    fn review_next(&self) -> Result<Option<crate::domain::ReviewItem>>;
    fn review_verdict(&self, id: i64, verdict: &crate::domain::Verdict) -> Result<()>;
    /// Ask for a message to be copied into a lower-labelled room (#497); queued for a verdict, returns the queue id.
    fn request_crossing(&self, from: &str, seq: i64, to: &str) -> Result<i64>;
    /// The review queue with verdicts and outcomes, newest first; pointers, never text (#497).
    fn review_queue(&self, limit: usize) -> Result<Vec<crate::store::QueuedReview>>;
}

impl<S: Store> Review for Direct<S> {
    fn my_crossings(&self, limit: usize) -> Result<Vec<crate::domain::CrossingStatus>> {
        self.gateway.my_crossings(&self.principal, limit)
    }
    fn review_next(&self) -> Result<Option<crate::domain::ReviewItem>> {
        self.gateway.review_next(&self.principal)
    }
    fn review_verdict(&self, id: i64, verdict: &crate::domain::Verdict) -> Result<()> {
        self.gateway.review_verdict(&self.principal, id, verdict)
    }
    fn request_crossing(&self, from: &str, seq: i64, to: &str) -> Result<i64> {
        self.gateway
            .request_crossing(&self.principal, from, seq, to)
    }
    fn review_queue(&self, limit: usize) -> Result<Vec<crate::store::QueuedReview>> {
        self.gateway.review_queue(&self.principal, limit)
    }
}

impl Review for SharedTransport {
    forward! {
        my_crossings(limit: usize) -> Result<Vec<crate::domain::CrossingStatus>>;
        review_next() -> Result<Option<crate::domain::ReviewItem>>;
        request_crossing(from: &str, seq: i64, to: &str) -> Result<i64>;
        review_queue(limit: usize) -> Result<Vec<crate::store::QueuedReview>>;
    }

    fn review_verdict(&self, id: i64, verdict: &crate::domain::Verdict) -> Result<()> {
        let verdict = verdict.clone();
        self.call(move |t| t.review_verdict(id, &verdict))
    }
}
