//! The `ReviewStore` area of the store seam; `Store` in the parent module combines every area.

use crate::domain::Result;
use crate::store::{NewReview, QueuedReview, ReviewResolution};

pub trait ReviewStore {
    /// The review queue (#48). Queue a message for a judge, oldest first back out, and record one verdict per item.
    /// A message already waiting for the same label is not queued twice (the existing id is returned). The default
    /// store has no queue, and says so rather than dropping the item.
    fn enqueue_review(&self, _item: &NewReview, _now: i64) -> Result<i64> {
        Err(crate::domain::Error::Backend(
            "this store has no review queue".into(),
        ))
    }

    fn pending_reviews(&self, _limit: usize) -> Result<Vec<QueuedReview>> {
        Ok(Vec::new())
    }

    /// Waiting items after id `after`, oldest first, at most `limit`: one page of the queue, so a caller can walk all of
    /// it by age and not only its first page (#1736). The default reads the whole queue and cuts it; a store with a
    /// real queue answers from an index.
    fn pending_reviews_after(&self, after: i64, limit: usize) -> Result<Vec<QueuedReview>> {
        Ok(self
            .pending_reviews(usize::MAX >> 1)?
            .into_iter()
            .filter(|q| q.id > after)
            .take(limit)
            .collect())
    }

    /// How many crossings `requester` has waiting for a verdict (#1736). The default counts the whole queue.
    fn waiting_crossings_of(&self, requester: &str) -> Result<usize> {
        Ok(self
            .pending_reviews(usize::MAX >> 1)?
            .iter()
            .filter(|q| !q.to_room.is_empty() && q.requester == requester)
            .count())
    }

    /// Record the verdict word and a short detail for a pending item; false when it was not pending.
    fn resolve_review(&self, _resolution: &ReviewResolution, _now: i64) -> Result<bool> {
        Ok(false)
    }

    /// Keep a plain note on a waiting item saying why a judge deferred it (the rules' flag, #1413), for the reviewer to
    /// read. It holds no message text and is overwritten by the verdict's own detail; false/no-op when not waiting.
    fn set_review_flag(&self, _id: i64, _note: &str) -> Result<()> {
        Ok(())
    }

    /// Keep the text a `redact` verdict supplied, on a waiting item (#497).
    fn set_review_text(&self, _id: i64, _text: &str) -> Result<()> {
        Ok(())
    }

    /// Every item, newest first, resolved or not: what a listing shows.
    fn reviews(&self, _limit: usize) -> Result<Vec<QueuedReview>> {
        Ok(Vec::new())
    }

    /// Crossings whose verdict is allow or redact and has not been applied yet, oldest first.
    fn unapplied_reviews(&self, _limit: usize) -> Result<Vec<QueuedReview>> {
        Ok(Vec::new())
    }

    /// Record that a verdict was applied (`released`) or stopped by a floor (`blocked`).
    fn mark_review_applied(&self, _id: i64, _outcome: &str) -> Result<()> {
        Ok(())
    }

    /// The member's own crossings that start in `room` and have news it was not told yet (#679): one `waiting` notice
    /// while undecided, then one for the outcome (denied, released or blocked). Each is returned once per stage and
    /// remembered. The default store has none.
    fn crossing_notices(&self, _room: &str, _member: &str) -> Result<Vec<QueuedReview>> {
        Ok(Vec::new())
    }
}
