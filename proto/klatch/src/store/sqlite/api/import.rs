//! The SQLite side of the storage-level import (#1217).

use super::super::rows::attribution_text;
use super::super::{SqliteStore, be};
use crate::domain::{Error, Message, Posted, Result};
use crate::store::ImportStore;
use rusqlite::params;

/// The longest id accepted: the minted ids are 26 characters; the margin is for ids another backend minted.
const MAX_ID: usize = 64;

fn check(message: &Message) -> Result<()> {
    let id = &message.id;
    if id.is_empty() || id.len() > MAX_ID || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(Error::Invalid(
            "an imported message needs an id of letters and digits".into(),
        ));
    }
    if message.label.is_empty() {
        return Err(Error::Invalid("an imported message needs a label".into()));
    }
    Ok(())
}

impl ImportStore for SqliteStore {
    fn import_message(&self, room: &str, message: &Message) -> Result<Posted> {
        check(message)?;
        let label = message.label.as_str();
        self.immediate(|| {
            let (next, taken): (i64, bool) = self
                .conn
                .query_row(
                    "SELECT (SELECT COALESCE(MAX(seq),0)+1 FROM messages WHERE room=?1),
                            EXISTS(SELECT 1 FROM messages WHERE mid=?2)",
                    params![room, message.id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(be)?;
            if taken {
                return Err(Error::Invalid("that message id is already in the store".into()));
            }
            if message.seq != next {
                return Err(Error::Invalid(format!("the next seq of this room is {next}")));
            }
            let id = message.id.as_str();
            let body = self.seal_text((label, id, "body"), &message.body)?;
            let payload = match message.payload.as_deref() {
                Some(p) => Some(self.seal_text((label, id, "payload"), p)?),
                None => None,
            };
            self.exec(
                "INSERT INTO messages(room,seq,mid,sender,member,body,ts,attribution,label,kind,payload) VALUES(?1,?2,?3,?4,?4,?5,?6,?7,?8,?9,?10)",
                params![room, next, id, message.sender, body, message.ts, attribution_text(message.attribution), label, message.kind, payload],
            )?;
            Ok(Posted { id: message.id.clone(), seq: next })
        })
    }
}
