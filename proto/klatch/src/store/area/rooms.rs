//! The `RoomStore` area of the store seam; `Store` in the parent module combines every area.

use crate::domain::Result;
use crate::store::DeletedRooms;

pub trait RoomStore {
    /// Set or clear (`None`) a room's administrative state, `closed`; the room `*` carries `paused` for every room
    /// (`klatch admin`, R11). The defaults are errors (#485): a store that cannot keep a pause or a close must not say
    /// no room is paused.
    fn set_room_state(&self, _room: &str, _state: Option<&str>, _now: i64) -> Result<()> {
        super::unsupported("room state")
    }

    fn room_state(&self, _room: &str) -> Result<Option<String>> {
        super::unsupported("room state")
    }

    /// Delete every room whose name starts with `prefix` (matched literally), from every table keyed by room, and the
    /// label policy of those rooms. The record of refusals and the administrative audit are not part of a room and
    /// stay. An empty prefix deletes nothing. The default is an error: a deletion that did not happen must not say it did.
    fn delete_rooms_with_prefix(&self, _prefix: &str, _now: i64) -> Result<DeletedRooms> {
        super::unsupported("rooms to delete")
    }

    /// The label keys this principal holds (DESIGN section 32 item 8). Required, with no default (#485): a store that
    /// ignored them would keep sealed text in the clear.
    fn set_label_keys(&self, keys: crate::keys::LabelKeys);
}
