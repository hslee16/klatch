//! The message, member and room parts of the spy store.

use super::*;

impl MessageStore for SpyStore {
    fn join(&self, room: &str, member: &str, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.join(room, member, now)
    }

    fn post_as(&self, message: &NewMessage, now: i64) -> Result<Posted> {
        self.gate()?;
        self.inner.post_as(message, now)
    }

    fn read(&self, at: Seat, advance: bool, now: i64) -> Result<Vec<Message>> {
        self.gate()?;
        self.inner.read(at, advance, now)
    }

    fn look(&self, at: Seat, now: i64) -> Result<Vec<Message>> {
        self.gate()?;
        self.inner.look(at, now)
    }

    fn posts_since(&self, member: &str, since: i64) -> Result<Vec<i64>> {
        self.inner.posts_since(member, since)
    }

    fn hook_read(&self, at: Seat, now: i64) -> Result<Vec<crate::domain::Message>> {
        self.gate()?;
        self.inner.hook_read(at, now)
    }

    fn wait_poll(&self, at: Seat, now: i64) -> Result<Vec<crate::domain::Message>> {
        self.gate()?;
        self.inner.wait_poll(at, now)
    }

    fn wake_poll(&self, at: Seat, now: i64) -> Result<Vec<crate::domain::Message>> {
        self.inner.wake_poll(at, now)
    }

    fn wake_seen(&self, at: Seat, seq: Seq, now: i64) -> Result<()> {
        self.inner.wake_seen(at, seq, now)
    }

    fn wait_seen(&self, at: Seat, seq: Seq, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.wait_seen(at, seq, now)
    }

    fn tail(&self, room: &str, after_seq: i64, limit: usize) -> Result<Vec<Message>> {
        self.gate()?;
        self.inner.tail(room, after_seq, limit)
    }

    fn head(&self, room: &str) -> Result<i64> {
        self.gate()?;
        self.inner.head(room)
    }

    fn advance_cursor(&self, at: Seat, seq: Seq, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.advance_cursor(at, seq, now)
    }
}

impl MemberStore for SpyStore {
    fn members(&self, room: &str) -> Result<Vec<MemberInfo>> {
        self.gate()?;
        self.inner.members(room)
    }

    fn note_parent(&self, member: &str, parent: &str, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.note_parent(member, parent, now)
    }

    fn parent_of(&self, member: &str) -> Result<Option<String>> {
        self.gate()?;
        self.inner.parent_of(member)
    }

    fn set_muted(&self, at: Seat, muted: bool, now: i64) -> Result<()> {
        self.inner.set_muted(at, muted, now)
    }

    fn member_cursors(&self, room: &str) -> Result<Vec<crate::domain::MemberCursor>> {
        self.inner.member_cursors(room)
    }

    fn own_post_count(&self, room: &str, member: &str) -> Result<i64> {
        self.inner.own_post_count(room, member)
    }

    fn is_muted(&self, at: Seat) -> Result<bool> {
        self.inner.is_muted(at)
    }

    fn block_member(&self, room: &str, member: &str, now: i64) -> Result<()> {
        self.inner.block_member(room, member, now)
    }

    fn unblock_member(&self, room: &str, member: &str) -> Result<bool> {
        self.inner.unblock_member(room, member)
    }

    fn is_blocked(&self, room: &str, member: &str) -> Result<bool> {
        self.inner.is_blocked(room, member)
    }

    fn remove_member(&self, room: &str, member: &str, now: i64) -> Result<usize> {
        self.inner.remove_member(room, member, now)
    }

    fn set_status(&self, at: Seat, status: &str, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.set_status(at, status, now)
    }

    fn clear_done(&self, at: Seat, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.clear_done(at, now)
    }

    fn record_hook_run(&self, run: &HookEvent, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.record_hook_run(run, now)
    }

    fn hook_runs(&self, member: &str) -> Result<Vec<crate::domain::HookRun>> {
        self.gate()?;
        self.inner.hook_runs(member)
    }

    fn prune_members(&self, older_than: i64, now: i64) -> Result<usize> {
        self.gate()?;
        self.inner.prune_members(older_than, now)
    }

    fn park(&self, member: &str, parked: &crate::domain::Parked, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.park(member, parked, now)
    }

    fn unpark(&self, member: &str, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.unpark(member, now)
    }

    fn note_member_mode(&self, member: &str, mode: &str, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.note_member_mode(member, mode, now)
    }

    fn trail_of(&self, member: &str) -> Result<crate::domain::Trail> {
        self.gate()?;
        self.inner.trail_of(member)
    }

    fn save_trail(&self, member: &str, trail: &crate::domain::Trail, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.save_trail(member, trail, now)
    }

    fn clear_trail(&self, member: &str) -> Result<()> {
        self.gate()?;
        self.inner.clear_trail(member)
    }

    fn put_machine_checkpoint(
        &self,
        at: (&str, &str),
        body: &str,
        last: bool,
        now: i64,
    ) -> Result<bool> {
        self.gate()?;
        self.inner.put_machine_checkpoint(at, body, last, now)
    }

    fn put_machine_status(
        &self,
        at: Seat,
        status: (&str, &str),
        owned: &[&str],
        when: (i64, i64),
    ) -> Result<bool> {
        self.gate()?;
        self.inner.put_machine_status(at, status, owned, when)
    }

    fn machine_checkpoints(&self, room: &str) -> Result<Vec<crate::domain::MachineCheckpoint>> {
        self.gate()?;
        self.inner.machine_checkpoints(room)
    }

    fn member_modes(&self) -> Result<Vec<(String, String, i64)>> {
        self.gate()?;
        self.inner.member_modes()
    }

    fn note_session(&self, session_id: &str, member: &str, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.note_session(session_id, member, now)
    }

    fn session_of_member(&self, member: &str) -> Result<Option<String>> {
        self.gate()?;
        self.inner.session_of_member(member)
    }

    fn member_of_session(&self, session_id: &str) -> Result<Option<String>> {
        self.gate()?;
        self.inner.member_of_session(session_id)
    }

    fn member_scope(&self, member: &str) -> Result<Option<String>> {
        self.gate()?;
        self.inner.member_scope(member)
    }

    fn set_member_scope(&self, member: &str, scope: &str, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.set_member_scope(member, scope, now)
    }

    fn member_attrs(&self, member: &str) -> Result<Option<crate::domain::MemberAttrs>> {
        self.gate()?;
        self.inner.member_attrs(member)
    }

    fn note_member_attrs(
        &self,
        member: &str,
        attrs: &crate::domain::MemberAttrs,
        now: i64,
    ) -> Result<()> {
        self.gate()?;
        self.inner.note_member_attrs(member, attrs, now)
    }

    fn members_in_repository(&self, repository: &str) -> Result<Vec<String>> {
        self.gate()?;
        self.inner.members_in_repository(repository)
    }

    fn rooms_of(&self, member: &str) -> Result<Vec<String>> {
        self.gate()?;
        self.inner.rooms_of(member)
    }
}

impl RoomStore for SpyStore {
    fn features(&self) -> crate::store::Features {
        self.inner.features()
    }
    fn set_room_state(&self, room: &str, state: Option<&str>, now: i64) -> Result<()> {
        self.inner.set_room_state(room, state, now)
    }

    fn room_state(&self, room: &str) -> Result<Option<String>> {
        self.inner.room_state(room)
    }

    fn set_label_keys(&self, keys: crate::keys::LabelKeys) {
        self.inner.set_label_keys(keys);
    }

    fn delete_rooms_with_prefix(
        &self,
        prefix: &str,
        now: i64,
    ) -> Result<crate::store::DeletedRooms> {
        self.inner.delete_rooms_with_prefix(prefix, now)
    }
}
