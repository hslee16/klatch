//! The `MemberStore` area of the store seam; `Store` in the parent module combines every area.

use crate::domain::{HookRun, MemberInfo, Parked, Result};
use crate::store::{HookEvent, Seat};

pub trait MemberStore {
    fn members(&self, room: &str) -> Result<Vec<MemberInfo>>;

    /// Remember that harness session `session_id` is member `member` (replacing an earlier entry), so a hook that
    /// runs after the harness has dropped its own record can still find its member. The default remembers nothing.
    fn note_session(&self, _session_id: &str, _member: &str, _now: i64) -> Result<()> {
        Ok(())
    }

    /// The harness session that holds a member name, if any: the most recent one noted for it.
    fn session_of_member(&self, _member: &str) -> Result<Option<String>> {
        Ok(None)
    }

    /// The member remembered for a harness session, if any.
    fn member_of_session(&self, _session_id: &str) -> Result<Option<String>> {
        Ok(None)
    }

    /// Remove members of every room not seen for `older_than` seconds, with their per-member bookkeeping (claims
    /// seen, reported asks, hook runs). Messages and claims stay. Returns how many members were removed. The default
    /// removes nothing.
    fn prune_members(&self, _older_than: i64, _now: i64) -> Result<usize> {
        Ok(0)
    }

    /// Park a member (DESIGN section 32 item 45): until `parked.back_by` its live claims and ask leases run at
    /// least that long, it stays on the board however long since it was seen, and pruning spares it. Replaces an
    /// earlier park. The default stores nothing.
    fn park(&self, _member: &str, _parked: &Parked, _now: i64) -> Result<()> {
        Ok(())
    }

    /// End a member's park on a sign of life: claims and leases it held that run past a normal lease are cut back
    /// to one from `now`. Does nothing for a member that is not parked.
    fn unpark(&self, _member: &str, _now: i64) -> Result<()> {
        Ok(())
    }

    /// Record the member's status in a room (replacing any earlier one); joins the member if needed. The default
    /// stores nothing.
    fn set_status(&self, _at: Seat, _status: &str, _now: i64) -> Result<()> {
        Ok(())
    }

    /// The scope a member declared or was given, if any. The default knows none.
    fn member_scope(&self, _member: &str) -> Result<Option<String>> {
        Ok(None)
    }

    /// Remember a member's scope (replacing an earlier one). The default stores nothing.
    fn set_member_scope(&self, _member: &str, _scope: &str, _now: i64) -> Result<()> {
        Ok(())
    }

    /// Remember that `parent` is the member whose session started `member`'s; an empty `parent` records that it was
    /// looked for and there is none. The first one noted is kept. The default stores nothing.
    fn note_parent(&self, _member: &str, _parent: &str, _now: i64) -> Result<()> {
        Ok(())
    }

    /// What was noted for `member`: `None` if nothing, `Some("")` if it was looked for and there is no parent.
    fn parent_of(&self, _member: &str) -> Result<Option<String>> {
        Ok(None)
    }

    /// Mute or unmute a room for a member: a muted member's hook shows only messages that mention it (#20). The
    /// default stores nothing.
    fn set_muted(&self, _at: Seat, _muted: bool, _now: i64) -> Result<()> {
        Ok(())
    }

    /// Whether the member muted the room. The default says no.
    fn is_muted(&self, _at: Seat) -> Result<bool> {
        Ok(false)
    }

    /// Refuse every operation of `member` in `room` (or in every room for `*`) until it is allowed again.
    fn block_member(&self, _room: &str, _member: &str, _now: i64) -> Result<()> {
        Ok(())
    }

    /// Lift a block; whether there was one.
    fn unblock_member(&self, _room: &str, _member: &str) -> Result<bool> {
        Ok(false)
    }

    /// Whether `member` is blocked in `room`, by a block on that room or on every room.
    fn is_blocked(&self, _room: &str, _member: &str) -> Result<bool> {
        Ok(false)
    }

    /// Take `member` out of `room` and release its claims there; returns how many claims were released.
    fn remove_member(&self, _room: &str, _member: &str, _now: i64) -> Result<usize> {
        Ok(0)
    }

    /// Record that a hook event ran for `member`: the time, and how many bytes it printed (never the content).
    /// The default records nothing.
    fn record_hook_run(&self, _run: &HookEvent, _now: i64) -> Result<()> {
        Ok(())
    }

    /// The latest run of each hook event recorded for `member`, ordered by event name.
    fn hook_runs(&self, _member: &str) -> Result<Vec<HookRun>> {
        Ok(Vec::new())
    }

    /// Remember the permission mode a member's session last reported in a hook input (#553); the last report wins.
    /// The default remembers nothing.
    fn note_member_mode(&self, _member: &str, _mode: &str, _now: i64) -> Result<()> {
        Ok(())
    }

    /// Every member's last reported permission mode as (member, mode, time), ordered by member.
    fn member_modes(&self) -> Result<Vec<(String, String, i64)>> {
        Ok(Vec::new())
    }

    /// Rooms a member has joined, ordered by name.
    fn rooms_of(&self, member: &str) -> Result<Vec<String>>;
}
