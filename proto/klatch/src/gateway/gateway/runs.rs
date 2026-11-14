//! Run-scoped namespaces (#122). A software factory's run owns the rooms named `run.<id>.<name>`. A launcher creates
//! the run and ends it; ending it removes those rooms and everything keyed by them, after an optional archive has seen
//! them. A room of a run that is not open is refused, so a stray worker cannot bring a torn-down run back, and a room
//! that only looks like a run room (no id, no name, an unknown id) is reserved. The record of refusals and the
//! administrative audit are not part of a run and stay.

use super::*;
use crate::domain::Message;
use crate::store::DeletedRooms;

const RUN_PREFIX: &str = "run.";

/// The prefix of every room of run `id`.
pub fn run_prefix(id: &str) -> String {
    format!("{RUN_PREFIX}{id}.")
}

/// A run id: 1 to 32 characters of letters, digits, dash and underscore (a dot would end it).
pub fn valid_run_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

/// A run as an administrator sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunInfo {
    pub id: String,
    pub created: i64,
    pub rooms: usize,
}

/// What the archive callback gets before a run's rooms are deleted: every room and its messages.
pub type Archive<'a> = &'a mut dyn FnMut(&[(String, Vec<Message>)]) -> Result<()>;

impl<S: Store> Gateway<S> {
    /// Refuse a room of a run that is not open, and a room named like a run room that is not one.
    pub(super) fn check_run(&self, room: &str) -> Result<()> {
        let Some(rest) = room.strip_prefix(RUN_PREFIX) else {
            return Ok(());
        };
        let reserved = |why: &str| {
            Err(Error::Refused {
                rule: "run-unknown".into(),
                reason: format!("room {room} {why}"),
            })
        };
        let Some((id, name)) = rest.split_once('.') else {
            return reserved("is reserved: a run room is run.<id>.<name>");
        };
        if !valid_run_id(id) || name.is_empty() {
            return reserved("is reserved: a run room is run.<id>.<name>");
        }
        if self.store.policy_get(&format!("run:{id}"))?.is_none() {
            return reserved(&format!("belongs to run {id}, which is not open"));
        }
        Ok(())
    }

    /// Open a run: its rooms may be used from now on.
    pub fn admin_run_create(&self, who: &Principal, id: &str) -> Result<()> {
        if !valid_run_id(id) {
            return Err(Error::Invalid(format!(
                "a run id is 1 to 32 characters of letters, digits, dash and underscore: {id:?}"
            )));
        }
        self.administer(who, ("run-create", id), |now| {
            let key = format!("run:{id}");
            if self.store.policy_get(&key)?.is_some() {
                return Err(Error::Invalid(format!("run {id} is already open")));
            }
            self.store.policy_set(&key, Some(&now.to_string()), now)?;
            Ok(String::new())
        })
    }

    /// End a run: `archive` sees every room's messages first (a failure there stops everything and the run stays
    /// open), then the rooms are deleted and the run closed.
    pub fn admin_run_end(
        &self,
        who: &Principal,
        id: &str,
        mut archive: Option<Archive>,
    ) -> Result<DeletedRooms> {
        if !valid_run_id(id) {
            return Err(Error::Invalid(format!("bad run id: {id:?}")));
        }
        let mut gone = DeletedRooms::default();
        self.administer(who, ("run-end", id), |now| {
            let key = format!("run:{id}");
            if self.store.policy_get(&key)?.is_none() {
                return Err(Error::NotFound(format!("run {id} is not open")));
            }
            let prefix = run_prefix(id);
            if let Some(archive) = archive.as_mut() {
                let mut rooms = Vec::new();
                for room in self.store.rooms_with_prefix(&prefix)? {
                    let mut all = Vec::new();
                    let mut after = 0;
                    loop {
                        let page = self.store.tail(&room.name, after, 5000)?;
                        let Some(last) = page.last().map(|m| m.seq) else {
                            break;
                        };
                        after = last;
                        all.extend(page);
                    }
                    rooms.push((room.name, all));
                }
                archive(&rooms)?;
            }
            gone = self.store.delete_rooms_with_prefix(&prefix, now)?;
            self.store.policy_set(&key, None, now)?;
            Ok(format!(
                "{} room(s), {} message(s) removed",
                gone.rooms, gone.messages
            ))
        })?;
        Ok(gone)
    }

    /// Every open run and how many rooms it has.
    pub fn admin_runs(&self, who: &Principal) -> Result<Vec<RunInfo>> {
        check_member(who)?;
        self.guard.check(who, &Operation::admin("runs", ""))?;
        let mut out = Vec::new();
        for (key, value) in self.store.policy_list("run:")? {
            let id = key.trim_start_matches("run:").to_string();
            out.push(RunInfo {
                rooms: self.store.rooms_with_prefix(&run_prefix(&id))?.len(),
                created: value.parse().unwrap_or(0),
                id,
            });
        }
        Ok(out)
    }
}
