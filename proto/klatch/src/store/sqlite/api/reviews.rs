//! The SQLite side of the `ReviewStore` area.

use super::super::SqliteStore;
use crate::domain::Result;
use crate::store::{NewReview, QueuedReview};
use crate::store::{ReviewResolution, ReviewStore};

impl ReviewStore for SqliteStore {
    fn enqueue_review(&self, item: &NewReview, now: i64) -> Result<i64> {
        self.enqueue_review_row(item, now)
    }

    fn pending_reviews(&self, limit: usize) -> Result<Vec<QueuedReview>> {
        self.pending_review_rows(limit)
    }

    fn set_review_text(&self, id: i64, text: &str) -> Result<()> {
        self.set_review_text_row(id, text)
    }

    fn reviews(&self, limit: usize) -> Result<Vec<QueuedReview>> {
        self.review_rows("1=1 ORDER BY id DESC", limit)
    }

    fn unapplied_reviews(&self, limit: usize) -> Result<Vec<QueuedReview>> {
        self.review_rows(
            "to_room<>'' AND verdict IN ('allow','redact') AND applied='' ORDER BY id",
            limit,
        )
    }

    fn mark_review_applied(&self, id: i64, outcome: &str) -> Result<()> {
        self.mark_review_applied_row(id, outcome)
    }

    fn resolve_review(&self, resolution: &ReviewResolution, now: i64) -> Result<bool> {
        self.resolve_review_row(resolution, now)
    }
}
