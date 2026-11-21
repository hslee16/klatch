//! The tasks section of the board.

use super::*;

impl<S: Store> Gateway<S> {
    /// The open tasks of a room. A task whose owner is not present (hidden, ended, pruned), not parked and holds no
    /// live claim is orphaned: it stays visible with its last checkpoint for the next helper.
    pub(super) fn board_tasks(
        &self,
        room: &str,
        now: i64,
        members: &[crate::domain::BoardMember],
    ) -> Result<Vec<crate::domain::Task>> {
        let holders: Vec<String> = self
            .store
            .claims(room, now)?
            .into_iter()
            .map(|c| c.owner)
            .collect();
        // the task words are read by prefix at any depth, not from the tail window: a task can outlive the window
        let words = self.store.words(room, &crate::domain::TASK_WORDS)?;
        let mut tasks = crate::domain::open_tasks(&words);
        for t in &mut tasks {
            let present = members.iter().any(|m| {
                m.name == t.owner && !m.status.as_deref().is_some_and(|s| s.starts_with("done"))
            });
            t.orphaned = !present && !holders.contains(&t.owner);
        }
        Ok(tasks)
    }
}
