//! The SQLite side of the `RoomStore` area.

use super::super::{SqliteStore, be};
use crate::domain::Result;
use crate::store::RoomStore;
use rusqlite::params;

impl RoomStore for SqliteStore {
    fn delete_rooms_with_prefix(
        &self,
        prefix: &str,
        _now: i64,
    ) -> Result<crate::store::DeletedRooms> {
        self.do_delete_rooms_with_prefix(prefix, _now)
    }

    fn set_label_keys(&self, keys: crate::keys::LabelKeys) {
        *self.labels.borrow_mut() = keys;
    }

    fn set_room_state(&self, room: &str, state: Option<&str>, now: i64) -> Result<()> {
        match state {
            Some(st) => self.conn.execute(
                "INSERT INTO room_state(room,state,ts) VALUES(?1,?2,?3)
                 ON CONFLICT(room) DO UPDATE SET state=?2, ts=?3",
                params![room, st, now],
            ),
            None => self
                .conn
                .execute("DELETE FROM room_state WHERE room=?1", params![room]),
        }
        .map(|_| ())
        .map_err(be)
    }

    fn room_state(&self, room: &str) -> Result<Option<String>> {
        self.optional(
            "SELECT state FROM room_state WHERE room=?1",
            params![room],
            |r| r.get(0),
        )
    }
}
