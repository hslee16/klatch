//! The `RoomStore` area of the store seam; `Store` in the parent module combines every area.

use crate::domain::Result;
use crate::store::DeletedRooms;

pub trait RoomStore {
    /// Set or clear (`None`) a room's administrative state, `closed`; the room `*` carries `paused` for every room
    /// (`klatch admin`, R11). The default remembers nothing.
    fn set_room_state(&self, _room: &str, _state: Option<&str>, _now: i64) -> Result<()> {
        Ok(())
    }

    fn room_state(&self, _room: &str) -> Result<Option<String>> {
        Ok(None)
    }

    /// Delete every room whose name starts with `prefix` (matched literally), from every table keyed by room, and the
    /// label policy of those rooms. The record of refusals and the administrative audit are not part of a room and
    /// stay. An empty prefix deletes nothing. The default deletes nothing.
    fn delete_rooms_with_prefix(&self, _prefix: &str, _now: i64) -> Result<DeletedRooms> {
        Ok(DeletedRooms::default())
    }

    /// The label keys this principal holds (DESIGN section 32 item 8). The default ignores them.
    fn set_label_keys(&self, _keys: crate::keys::LabelKeys) {}
}
