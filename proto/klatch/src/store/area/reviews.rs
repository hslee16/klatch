//! The `ReviewStore` area of the store seam; `Store` in the parent module combines every area.

use crate::domain::Result;
use crate::store::{NewReview, QueuedReview, ReviewResolution};

pub trait ReviewStore {
    /// The review queue (#48). Queue a message for a judge, oldest first back out, and record one verdict per item.
    /// A crossing already waiting for the same message, label, requester and target room is not queued twice (the existing
    /// id is returned); another member's, or another room's, is its own item (#1743). The default
    /// store has no queue, and says so rather than dropping the item.
    fn enqueue_review(&self, _item: &NewReview, _now: i64) -> Result<i64> {
        Err(crate::domain::Error::Backend(
            "this store has no review queue".into(),
        ))
    }

    /// Queue a crossing unless its requester already has `cap` waiting; `None` when the cap stopped it (#1736). A
    /// message already waiting for the label returns its id whatever the cap, since it adds nothing. A store with a real
    /// queue counts and inserts in one transaction, so two processes of one member cannot both take the last place; this
    /// default looks for a waiting duplicate (same message, label, requester and target room), counts and then queues,
    /// and is not atomic.
    fn enqueue_review_capped(&self, item: &NewReview, now: i64, cap: usize) -> Result<Option<i64>> {
        let waiting = self.pending_reviews(usize::MAX >> 1)?;
        if let Some(same) = waiting.iter().find(|r| {
            (r.room.as_str(), r.seq, r.to_label.as_str()) == (item.room, item.seq, item.to_label)
                && (r.requester.as_str(), r.to_room.as_str()) == (item.requester, item.to_room)
        }) {
            return Ok(Some(same.id));
        }
        if self.waiting_crossings_of(item.requester)? >= cap {
            return Ok(None);
        }
        self.enqueue_review(item, now).map(Some)
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

    /// Keep a plain note on a waiting item saying why a judge deferred it (a judge's flag, #1413), for the reviewer to
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
