//! Whole-room deletion, member pruning and member removal.

use super::*;

impl SqliteStore {
    pub(super) fn do_delete_rooms_with_prefix(
        &self,
        prefix: &str,
        _now: i64,
    ) -> Result<crate::store::DeletedRooms> {
        if prefix.is_empty() {
            return Ok(crate::store::DeletedRooms::default());
        }
        let n = prefix.chars().count() as i64;
        self.immediate(|| {
            let rooms: Vec<String> = self.rows(
                "SELECT DISTINCT room FROM members WHERE substr(room,1,?2)=?1
                 UNION SELECT DISTINCT room FROM messages WHERE substr(room,1,?2)=?1",
                params![prefix, n],
                |r| r.get(0),
            )?;
            let messages: i64 = self
                .conn
                .query_row(
                    "SELECT count(*) FROM messages WHERE substr(room,1,?2)=?1",
                    params![prefix, n],
                    |r| r.get(0),
                )
                .map_err(be)?;
            // every table that is keyed by a room, found from the schema so a table added later is not forgotten;
            // the record of refusals is an audit trail and stays
            let tables: Vec<String> = self.rows(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
                params![],
                |r| r.get(0),
            )?;
            for t in tables.iter().filter(|t| t.as_str() != "refusals") {
                let has_room: bool = self
                    .rows(&format!("PRAGMA table_info(\"{t}\")"), params![], |r| {
                        r.get::<_, String>(1)
                    })?
                    .iter()
                    .any(|c| c == "room");
                if has_room {
                    self.exec(
                        &format!("DELETE FROM \"{t}\" WHERE substr(room,1,?2)=?1"),
                        params![prefix, n],
                    )?;
                }
            }
            self.exec(
                "DELETE FROM policy WHERE substr(key,1,?2)=?1",
                params![
                    format!("label:{prefix}"),
                    (prefix.chars().count() + 6) as i64
                ],
            )?;
            Ok(crate::store::DeletedRooms {
                rooms: rooms.len(),
                messages: messages as usize,
            })
        })
    }

    pub(super) fn do_prune_members(&self, older_than: i64, now: i64) -> Result<usize> {
        self.immediate(|| {
            let cutoff = now - older_than;
            // A member that may return (it holds a live claim) keeps its rows, so a session resumed after days
            // finds its read cursor; later sources of "may return" (parked, open task) join this list.
            let keep = self.members_that_may_return(now)?;
            let kept = |name: &str| keep.iter().any(|k| k == name);
            self.prune_sessions(cutoff, &kept)?;
            let (removed, names) = self.prune_member_rows(cutoff, &kept)?;
            for name in names {
                self.forget_departed(&name)?;
            }
            Ok(removed)
        })
    }

    /// Delete the sessions not seen since `cutoff` whose member is not spared.
    fn prune_sessions(&self, cutoff: i64, kept: &dyn Fn(&str) -> bool) -> Result<()> {
        let old_sessions: Vec<(String, String)> = self.rows(
            "SELECT session_id,member FROM sessions WHERE ts<?1",
            params![cutoff],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        for (id, _) in old_sessions.iter().filter(|(_, m)| !kept(m)) {
            self.exec("DELETE FROM sessions WHERE session_id=?1", params![id])?;
        }
        Ok(())
    }

    /// Delete the room memberships not seen since `cutoff` whose member is not spared: how many rows went, and
    /// the names that lost one.
    fn prune_member_rows(
        &self,
        cutoff: i64,
        kept: &dyn Fn(&str) -> bool,
    ) -> Result<(usize, Vec<String>)> {
        let old: Vec<(String, String)> = self.rows(
            "SELECT room,name FROM members WHERE last_seen<?1",
            params![cutoff],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let mut names: Vec<String> = Vec::new();
        let mut removed = 0;
        for (room, name) in old.iter().filter(|(_, n)| !kept(n)) {
            removed += self.exec(
                "DELETE FROM members WHERE room=?1 AND name=?2",
                params![room, name],
            )?;
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
        Ok((removed, names))
    }

    /// A name that no longer belongs to any room takes its per-member rows with it.
    fn forget_departed(&self, name: &str) -> Result<()> {
        let still: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM members WHERE name=?1",
                params![name],
                |r| r.get(0),
            )
            .map_err(be)?;
        if still > 0 {
            return Ok(());
        }
        for sql in [
            "DELETE FROM claims_seen WHERE member=?1",
            "DELETE FROM asks_reported WHERE member=?1",
            "DELETE FROM asks_escalated WHERE member=?1",
            "DELETE FROM hook_runs WHERE member=?1",
            "DELETE FROM member_mode WHERE member=?1",
            "DELETE FROM claim_waits WHERE member=?1",
            "DELETE FROM member_parked WHERE member=?1",
        ] {
            self.exec(sql, params![name])?;
        }
        Ok(())
    }

    pub(super) fn do_remove_member(&self, room: &str, member: &str, now: i64) -> Result<usize> {
        self.immediate(|| {
            let released: usize = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM claims WHERE room=?1 AND owner=?2 AND expires>?3",
                    params![room, member, now],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(be)? as usize;
            for sql in [
                "DELETE FROM claims WHERE room=?1 AND owner=?2",
                "DELETE FROM members WHERE room=?1 AND name=?2",
            ] {
                self.exec(sql, params![room, member])?;
            }
            Ok(released)
        })
    }
}
