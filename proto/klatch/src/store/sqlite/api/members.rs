//! The SQLite side of the `MemberStore` area.

use super::super::{SqliteStore, be};
use crate::domain::{HookRun, MemberAttrs, MemberInfo, Parked, Result};
use crate::store::MemberStore;
use crate::store::{HookEvent, Seat};
use rusqlite::params;

impl MemberStore for SqliteStore {
    fn note_session(&self, session_id: &str, member: &str, now: i64) -> Result<()> {
        self.write(
            "INSERT INTO sessions(session_id,member,ts) VALUES(?1,?2,?3)
             ON CONFLICT(session_id) DO UPDATE SET member=?2, ts=?3",
            params![session_id, member, now],
        )
    }

    fn session_of_member(&self, member: &str) -> Result<Option<String>> {
        self.optional(
            "SELECT session_id FROM sessions WHERE member=?1 ORDER BY ts DESC LIMIT 1",
            params![member],
            |r| r.get(0),
        )
    }

    fn member_of_session(&self, session_id: &str) -> Result<Option<String>> {
        self.optional(
            "SELECT member FROM sessions WHERE session_id=?1",
            params![session_id],
            |r| r.get(0),
        )
    }

    fn prune_members(&self, older_than: i64, now: i64) -> Result<usize> {
        self.do_prune_members(older_than, now)
    }

    fn member_scope(&self, member: &str) -> Result<Option<String>> {
        self.optional(
            "SELECT scope FROM member_scope WHERE member=?1",
            params![member],
            |r| r.get(0),
        )
    }

    fn set_member_scope(&self, member: &str, scope: &str, now: i64) -> Result<()> {
        self.write(
            "INSERT INTO member_scope(member,scope,ts) VALUES(?1,?2,?3)
             ON CONFLICT(member) DO UPDATE SET scope=?2, ts=?3",
            params![member, scope, now],
        )
    }

    fn member_attrs(&self, member: &str) -> Result<Option<MemberAttrs>> {
        self.optional(
            "SELECT member_id,repository,directory FROM member_attrs WHERE member=?1",
            params![member],
            |r| {
                Ok(MemberAttrs::new(
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
    }

    fn note_member_attrs(&self, member: &str, attrs: &MemberAttrs, now: i64) -> Result<()> {
        self.write(
            "INSERT INTO member_attrs(member,member_id,repository,directory,ts) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(member) DO UPDATE SET repository=?3, directory=?4, ts=?5",
            params![member, attrs.member_id, attrs.repository, attrs.directory, now],
        )
    }

    fn members_in_repository(&self, repository: &str) -> Result<Vec<String>> {
        if repository.is_empty() {
            return Ok(Vec::new());
        }
        self.rows(
            "SELECT member FROM member_attrs WHERE repository=?1 ORDER BY member",
            params![repository],
            |r| r.get(0),
        )
    }

    fn set_status(&self, at: Seat, status: &str, now: i64) -> Result<()> {
        self.immediate(|| {
            self.touch(at.room, at.member, now)?;
            self.exec(
                "UPDATE members SET status=?3, status_ts=?4 WHERE room=?1 AND name=?2",
                params![at.room, at.member, status, now],
            )
            .map(|_| ())
        })
    }

    fn clear_done(&self, at: Seat, _now: i64) -> Result<()> {
        // no `touch`: this runs on a post or claim that already refreshed presence, and a member that is not in
        // the room has no status to clear
        self.exec(
            "UPDATE members SET status=NULL, status_ts=0 WHERE room=?1 AND name=?2 AND status LIKE 'done%'",
            params![at.room, at.member],
        )
        .map(|_| ())
    }

    fn record_hook_run(&self, run: &HookEvent, now: i64) -> Result<()> {
        let HookEvent {
            member,
            event,
            bytes,
        } = *run;
        self.write(
            "INSERT INTO hook_runs(member,event,last_ts,runs,last_bytes) VALUES(?1,?2,?3,1,?4)
             ON CONFLICT(member,event) DO UPDATE SET last_ts=?3, runs=runs+1, last_bytes=?4",
            params![member, event, now, bytes],
        )
    }

    fn hook_runs(&self, member: &str) -> Result<Vec<HookRun>> {
        self.rows(
            "SELECT event,last_ts,runs,last_bytes FROM hook_runs WHERE member=?1 ORDER BY event",
            params![member],
            |r| {
                Ok(HookRun {
                    event: r.get(0)?,
                    last_ts: r.get(1)?,
                    runs: r.get(2)?,
                    last_bytes: r.get(3)?,
                })
            },
        )
    }

    fn note_member_mode(&self, member: &str, mode: &str, now: i64) -> Result<()> {
        self.write(
            "INSERT INTO member_mode(member,mode,ts) VALUES(?1,?2,?3)
             ON CONFLICT(member) DO UPDATE SET mode=?2, ts=?3",
            params![member, mode, now],
        )
    }

    fn member_modes(&self) -> Result<Vec<(String, String, i64)>> {
        self.rows(
            "SELECT member,mode,ts FROM member_mode ORDER BY member",
            params![],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
    }

    fn members(&self, room: &str) -> Result<Vec<MemberInfo>> {
        self.rows(
            "SELECT m.name,m.last_seen,m.status,NULLIF(p.parent,''),k.why,k.back_by,m.joined,m.status_ts FROM members m
             LEFT JOIN member_parent p ON p.member=m.name
             LEFT JOIN member_parked k ON k.member=m.name WHERE m.room=?1 ORDER BY m.name",
            params![room],
            |r| {
                let why: Option<String> = r.get(4)?;
                let back_by: Option<i64> = r.get(5)?;
                Ok(MemberInfo {
                    status_ts: r.get::<_, Option<i64>>(7)?.unwrap_or(0),
                    name: r.get(0)?,
                    joined: r.get::<_, Option<i64>>(6)?.unwrap_or(0),
                    last_seen: r.get(1)?,
                    status: r.get(2)?,
                    parent: r.get(3)?,
                    parked: back_by.map(|back_by| Parked {
                        why: why.unwrap_or_default(),
                        back_by,
                    }),
                    kind: None,
                    place: None,
                    mode: None,
                })
            },
        )
    }

    fn park(&self, member: &str, parked: &Parked, now: i64) -> Result<()> {
        self.do_park(member, parked, now)
    }

    fn unpark(&self, member: &str, now: i64) -> Result<()> {
        self.do_unpark(member, now)
    }

    fn note_parent(&self, member: &str, parent: &str, now: i64) -> Result<()> {
        self.write(
            "INSERT OR IGNORE INTO member_parent(member,parent,ts) VALUES(?1,?2,?3)",
            params![member, parent, now],
        )
    }

    fn parent_of(&self, member: &str) -> Result<Option<String>> {
        self.optional(
            "SELECT parent FROM member_parent WHERE member=?1",
            params![member],
            |r| r.get(0),
        )
    }

    fn set_muted(&self, at: Seat, muted: bool, now: i64) -> Result<()> {
        self.update_member(
            (at.room, at.member, now),
            "UPDATE members SET muted=?3 WHERE room=?1 AND name=?2",
            &i64::from(muted),
        )
    }

    fn member_cursors(&self, room: &str) -> Result<Vec<crate::domain::MemberCursor>> {
        self.rows(
            "SELECT name,COALESCE(last_seq,0),hook_seq,muted FROM members WHERE room=?1 ORDER BY name",
            params![room],
            |r| {
                Ok(crate::domain::MemberCursor {
                    name: r.get(0)?,
                    read: r.get(1)?,
                    hook: r.get(2)?,
                    muted: r.get::<_, i64>(3)? != 0,
                })
            },
        )
    }

    fn is_muted(&self, at: Seat) -> Result<bool> {
        let muted = self.optional(
            "SELECT muted FROM members WHERE room=?1 AND name=?2",
            params![at.room, at.member],
            |r| r.get::<_, i64>(0),
        )?;
        Ok(muted.is_some_and(|m| m != 0))
    }

    fn block_member(&self, room: &str, member: &str, now: i64) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO member_blocks(room,member,ts) VALUES(?1,?2,?3)",
                params![room, member, now],
            )
            .map(|_| ())
            .map_err(be)
    }

    fn unblock_member(&self, room: &str, member: &str) -> Result<bool> {
        let n = self.exec(
            "DELETE FROM member_blocks WHERE room=?1 AND member=?2",
            params![room, member],
        )?;
        Ok(n > 0)
    }

    fn is_blocked(&self, room: &str, member: &str) -> Result<bool> {
        let n: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM member_blocks WHERE member=?2 AND (room=?1 OR room='*')",
                params![room, member],
                |r| r.get(0),
            )
            .map_err(be)?;
        Ok(n > 0)
    }

    fn remove_member(&self, room: &str, member: &str, now: i64) -> Result<usize> {
        self.do_remove_member(room, member, now)
    }

    fn rooms_of(&self, member: &str) -> Result<Vec<String>> {
        self.rows(
            "SELECT room FROM members WHERE name=?1 ORDER BY room",
            params![member],
            |r| r.get(0),
        )
    }
}
