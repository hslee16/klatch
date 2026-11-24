//! The storage-level import (#1217). It is deliberately not an area of `Store`: the gateway, the server and every
//! agent-facing path are bound by `Store`, so they cannot name it. Only code that holds a concrete store (a backend
//! migration, an archive restore) can.

use crate::domain::{Message, Posted, Result};

pub trait ImportStore {
    /// Append `message` to `room` exactly as it was kept elsewhere: its id, its seq, its time, its sender, attribution,
    /// label and handoff. It is refused (`Error::Invalid`, nothing written) when the id is malformed or already used
    /// anywhere in the store, when the label is empty, or when `message.seq` is not the room's next seq, so a replay
    /// in order keeps the log dense and ordered. It marks nobody present and moves no cursor: an import restores
    /// history, it is not a member acting. An importer must not import a body it read without the label key (a placeholder
    /// stands in for it), and `advisory`, `scope` and `refs` are not carried: the store leaves them unset on every message.
    fn import_message(&self, room: &str, message: &Message) -> Result<Posted>;
}
