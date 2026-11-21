//! The `tasks:` section of the board text.

use super::{Board, Now, render_list};

/// The `tasks:` section: only when there are open tasks.
pub(super) fn render(board: &Board, now: Now, out: &mut String) {
    let tasks: Vec<String> = board.tasks.iter().map(|t| t.line(now.0, false)).collect();
    render_list(out, "tasks:", &tasks, "tasks");
}
