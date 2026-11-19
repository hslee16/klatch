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

    /// Record the verdict word and a short detail for a pending item; false when it was not pending.
    fn resolve_review(&self, _resolution: &ReviewResolution, _now: i64) -> Result<bool> {
        Ok(false)
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
