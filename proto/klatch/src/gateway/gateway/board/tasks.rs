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
        self.attach_machine_checkpoints(room, now, &mut tasks)?;
        for t in &mut tasks {
            let present = members.iter().any(|m| {
                m.name == t.owner && !m.status.as_deref().is_some_and(|s| s.starts_with("done"))
            });
            t.orphaned = !present && !holders.contains(&t.owner);
        }
        Ok(tasks)
    }

    /// The machine checkpoints the hooks wrote (#1408), each shown with its owner's latest task, beneath the owner's
    /// own checkpoint. An owner with no open task gets a task of its own that stands for the checkpoint, so a
    /// member that never declared work still leaves where it stood. One older than a task is held (a week) is gone.
    fn attach_machine_checkpoints(
        &self,
        room: &str,
        now: i64,
        tasks: &mut Vec<crate::domain::Task>,
    ) -> Result<()> {
        for m in self.store.machine_checkpoints(room)? {
            if now - m.ts <= crate::domain::TASK_HOLD_SECS {
                attach_checkpoint(tasks, m);
            }
        }
        Ok(())
    }
}

/// Show the checkpoint `m` beneath its owner's latest task, or as a task of its own when the owner has none.
fn attach_checkpoint(tasks: &mut Vec<crate::domain::Task>, m: crate::domain::MachineCheckpoint) {
    let done = m.body.split("; ").any(|p| p == "status done");
    let machine = Some(crate::domain::Checkpoint {
        text: m.body,
        ts: m.ts,
    });
    let latest = tasks
        .iter_mut()
        .filter(|t| t.owner == m.member)
        .max_by_key(|t| t.last);
    match latest {
        Some(t) => t.machine = machine,
        // a member that said it is done leaves no stand-in task behind
        None if done => {}
        None => tasks.push(stand_in_task(m.member, m.ts, machine)),
    }
}

/// The task that stands for a machine checkpoint of a member that declared no work.
fn stand_in_task(
    owner: String,
    ts: i64,
    machine: Option<crate::domain::Checkpoint>,
) -> crate::domain::Task {
    crate::domain::Task {
        owner,
        title: "session".into(),
        what: String::new(),
        started: ts,
        last: ts,
        checkpoint: None,
        machine,
        undeclared: true,
        from: None,
        orphaned: false,
    }
}
