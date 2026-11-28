//! The SQLite side of the `ReviewStore` area.

use super::super::SqliteStore;
use super::super::review::UNAPPLIED_TAIL;
use crate::domain::Result;
use crate::store::{NewReview, QueuedReview};
use crate::store::{ReviewResolution, ReviewStore};

impl ReviewStore for SqliteStore {
    fn enqueue_review(&self, item: &NewReview, now: i64) -> Result<i64> {
        self.enqueue_review_row(item, now, None)
            .map(|id| id.unwrap_or_default())
    }

    fn enqueue_review_capped(&self, item: &NewReview, now: i64, cap: usize) -> Result<Option<i64>> {
        self.enqueue_review_row(item, now, Some(cap))
    }

    fn pending_reviews(&self, limit: usize) -> Result<Vec<QueuedReview>> {
        self.pending_review_rows(limit)
    }

    fn pending_reviews_after(&self, after: i64, limit: usize) -> Result<Vec<QueuedReview>> {
        self.pending_review_rows_after(after, limit)
    }

    fn waiting_crossings_of(&self, requester: &str) -> Result<usize> {
        self.waiting_crossing_count(requester)
    }

    fn set_review_text(&self, id: i64, text: &str) -> Result<()> {
        self.set_review_text_row(id, text)
    }

    fn set_review_flag(&self, id: i64, note: &str) -> Result<()> {
        self.set_review_flag_row(id, note)
    }

    fn reviews(&self, limit: usize) -> Result<Vec<QueuedReview>> {
        self.review_rows("1=1 ORDER BY id DESC", limit)
    }

    fn unapplied_reviews(&self, limit: usize) -> Result<Vec<QueuedReview>> {
        self.review_rows(UNAPPLIED_TAIL, limit)
    }

    fn mark_review_applied(&self, id: i64, outcome: &str) -> Result<()> {
        self.mark_review_applied_row(id, outcome)
    }

    fn crossing_notices(&self, room: &str, member: &str) -> Result<Vec<QueuedReview>> {
        self.crossing_notice_rows(room, member)
    }

    fn resolve_review(&self, resolution: &ReviewResolution, now: i64) -> Result<bool> {
        self.resolve_review_row(resolution, now)
    }
}
