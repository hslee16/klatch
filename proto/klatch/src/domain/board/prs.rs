//! The `prs:` section of the board text.

use super::{Board, Now, render_list};

/// The `prs:` section: only when there are open pull requests.
pub(super) fn render(board: &Board, now: Now, out: &mut String) {
    let prs: Vec<String> = board.prs.iter().map(|p| p.line(now.0)).collect();
    render_list(out, "prs:", &prs, "prs");
}
