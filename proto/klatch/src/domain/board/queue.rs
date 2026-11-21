//! The merge queue section of the board text (#864).

use super::{Board, Now, render_list};
use crate::domain::flat;

/// The `queue` section: only when there are entries; its heading says who holds `merge:main`.
pub(super) fn render(board: &Board, now: Now, out: &mut String) {
    let mut queue: Vec<String> = board.halt.iter().map(|h| h.line(now.0)).collect();
    queue.extend(board.queue.iter().map(|e| e.line(now.0)));
    let heading = match &board.merge_holder {
        Some(h) => format!("queue (merge:main held by {}):", flat(h)),
        None => "queue (merge:main free):".to_string(),
    };
    render_list(out, &heading, &queue, "queue entries");
}
