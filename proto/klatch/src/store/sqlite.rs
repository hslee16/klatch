use crate::domain::{ASK_LEASE_DEFAULT_SECS, CLAIM_DEFAULT_SECS};
use crate::domain::{
    AskRoom, AuditRecord, Claim, ClaimChanges, Error, HookRun, MemberInfo, Message, OpenAsk,
    Parked, Posted, RefusalRecord, ReleaseNote, ReleasedClaim, Result,
};
use crate::store::ids::new_id;
use crate::store::{
    ClaimRequest, HookEvent, NewAudit, NewMessage, NewRefusal, NewReview, QueuedReview, Release,
    Seat, Store,
};
use rusqlite::{Connection, params};

mod archive;
mod asks;
mod backup;
mod claims;
mod connect;
mod credentials;
mod exec;
mod housekeeping;
mod integrity;
mod leases;
mod messaging;
mod parking;
mod releases;
mod review;
mod rows;
mod schema;
mod seal;
mod tasks;
pub use archive::{
    ArchiveInfo, ArchiveState, archives_to_rekey, list_archives, rekey_archives, rekey_archives_in,
};
use connect::be;
pub use integrity::check_store;
use rows::{MESSAGE_COLUMNS, attribution_text};
pub use schema::SCHEMA_VERSION;
use schema::WAIT_WINDOW_SECS;

/// SQLite in WAL mode. Many readers, one writer at a time; a claim check-and-insert and a post are each one
/// `BEGIN IMMEDIATE` transaction so they are atomic across processes.
///
/// Data-model rules (DESIGN D6): every row carries a `tenant` (always `local` in v1) and a `member`
/// (until the `Identity` trait exists the member id is the member's name); message ids are globally unique
/// and separate from the dense per-room `seq`.
pub struct SqliteStore {
    conn: Connection,
    /// The label keys this principal holds (DESIGN section 32 item 8). A label it has no key for stays sealed.
    labels: std::cell::RefCell<crate::keys::LabelKeys>,
}

fn credential_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<crate::store::CredentialRecord> {
    Ok(crate::store::CredentialRecord {
        member: r.get(0)?,
        token_hash: r.get(1)?,
        clearance: r.get(2)?,
        parent: r.get(3)?,
        issued: r.get(4)?,
        expires: r.get(5)?,
        revoked: r.get::<_, i64>(6)? != 0,
    })
}

impl Store for SqliteStore {
    fn delete_rooms_with_prefix(
        &self,
        prefix: &str,
        _now: i64,
    ) -> Result<crate::store::DeletedRooms> {
        self.do_delete_rooms_with_prefix(prefix, _now)
    }

    fn release_anchored(&self, anchor: &str, now: i64) -> Result<Vec<(String, String, String)>> {
        self.do_release_anchored(anchor, now)
    }

    fn anchors_in_use(&self, now: i64) -> Result<Vec<String>> {
        self.rows(
            "SELECT DISTINCT anchor FROM claims WHERE anchor<>'' AND expires>?1 ORDER BY anchor",
            params![now],
            |r| r.get(0),
        )
    }

    fn issue_credential(&self, c: &crate::store::NewCredential, now: i64) -> Result<()> {
        self.do_issue_credential(c, now)
    }

    fn credential(&self, member: &str) -> Result<Option<crate::store::CredentialRecord>> {
        Ok(self
            .rows(
                "SELECT member,token_hash,clearance,parent,issued,expires,revoked FROM credentials WHERE member=?1",
                params![member],
                credential_from_row,
            )?
            .into_iter()
            .next())
    }

    fn revoke_credential(&self, member: &str, _now: i64) -> Result<bool> {
        Ok(self.exec(
            "UPDATE credentials SET revoked=1 WHERE member=?1 AND revoked=0",
            params![member],
        )? > 0)
    }

    fn credentials(&self) -> Result<Vec<crate::store::CredentialRecord>> {
        self.rows(
            "SELECT member,token_hash,clearance,parent,issued,expires,revoked FROM credentials ORDER BY member",
            params![],
            credential_from_row,
        )
    }

    fn set_label_keys(&self, keys: crate::keys::LabelKeys) {
        *self.labels.borrow_mut() = keys;
    }

    fn join(&self, room: &str, member: &str, now: i64) -> Result<()> {
        self.touch(room, member, now)
    }

    fn post_as(&self, message: &NewMessage, now: i64) -> Result<Posted> {
        self.do_post_as(message, now)
    }

    fn read(&self, at: Seat, advance: bool, now: i64) -> Result<Vec<Message>> {
        self.do_read(at, advance, now)
    }

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

    fn set_status(&self, at: Seat, status: &str, now: i64) -> Result<()> {
        self.update_member(
            (at.room, at.member, now),
            "UPDATE members SET status=?3 WHERE room=?1 AND name=?2",
            &status,
        )
    }

    fn wait_poll(&self, room: &str, member: &str, now: i64) -> Result<Vec<Message>> {
        self.do_wait_poll(room, member, now)
    }

    fn wait_seen(&self, at: Seat, seq: i64, now: i64) -> Result<()> {
        self.update_member(
            (at.room, at.member, now),
            "UPDATE members SET wait_seq=MAX(wait_seq,?3) WHERE room=?1 AND name=?2",
            &seq,
        )
    }

    fn wake_poll(&self, room: &str, member: &str, now: i64) -> Result<Vec<Message>> {
        self.do_wake_poll(room, member, now)
    }

    fn wake_seen(&self, at: Seat, seq: i64, now: i64) -> Result<()> {
        self.update_member(
            (at.room, at.member, now),
            "UPDATE members SET wake_seq=MAX(wake_seq,?3) WHERE room=?1 AND name=?2",
            &seq,
        )
    }

    fn rooms_with_prefix(&self, prefix: &str) -> Result<Vec<AskRoom>> {
        self.rows(
            "SELECT m.room, COUNT(DISTINCT m.name), MAX(m.joined),
                        COALESCE((SELECT MAX(x.ts) FROM messages x WHERE x.room=m.room), 0)
                 FROM members m WHERE substr(m.room,1,?2)=?1 GROUP BY m.room ORDER BY m.room",
            params![prefix, prefix.chars().count() as i64],
            |r| {
                let seen: i64 = r.get(2)?;
                let said: i64 = r.get(3)?;
                Ok(AskRoom {
                    name: r.get(0)?,
                    members: r.get::<_, i64>(1)? as usize,
                    last_activity: seen.max(said),
                })
            },
        )
    }

    fn lapsed_asks(&self, room: &str, member: &str, now: i64) -> Result<Vec<(OpenAsk, String)>> {
        self.immediate(|| {
            let mut out = Vec::new();
            for a in self.open_asks(room, now)? {
                if a.sender != member || a.claimed {
                    continue;
                }
                let Some((owner, expires)) = self
                    .ask_lease(room, a.seq)?
                    .filter(|(_, expires)| *expires <= now)
                else {
                    continue;
                };
                if self.report_lapse_once((room, member), a.seq, expires)? {
                    out.push((a, owner));
                }
            }
            Ok(out)
        })
    }

    fn stale_asks(&self, at: Seat, now: i64, after: i64) -> Result<Vec<OpenAsk>> {
        let Seat { room, member } = at;
        self.immediate(|| {
            let stale = self
                .open_asks(room, now)?
                .into_iter()
                .filter(|a| asks::is_stale(a, member, now, after))
                .collect();
            self.report_once("asks_reported", (room, member), stale)
        })
    }

    fn escalated_asks(&self, room: &str, member: &str, now: i64) -> Result<Vec<OpenAsk>> {
        self.immediate(|| {
            let escalated = self
                .open_asks(room, now)?
                .into_iter()
                .filter(|a| a.sender == member && a.no_capable_member())
                .collect();
            self.report_once("asks_escalated", (room, member), escalated)
        })
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

    fn hook_read(&self, room: &str, member: &str, now: i64) -> Result<Vec<Message>> {
        self.do_hook_read(room, member, now)
    }

    fn members(&self, room: &str) -> Result<Vec<MemberInfo>> {
        self.rows(
            "SELECT m.name,m.last_seen,m.status,NULLIF(p.parent,''),k.why,k.back_by,m.joined FROM members m
             LEFT JOIN member_parent p ON p.member=m.name
             LEFT JOIN member_parked k ON k.member=m.name WHERE m.room=?1 ORDER BY m.name",
            params![room],
            |r| {
                let why: Option<String> = r.get(4)?;
                let back_by: Option<i64> = r.get(5)?;
                Ok(MemberInfo {
                    name: r.get(0)?,
                    joined: r.get::<_, Option<i64>>(6)?.unwrap_or(0),
                    last_seen: r.get(1)?,
                    status: r.get(2)?,
                    parent: r.get(3)?,
                    parked: back_by.map(|back_by| Parked {
                        why: why.unwrap_or_default(),
                        back_by,
                    }),
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

    fn claim(&self, request: &ClaimRequest, now: i64) -> Result<()> {
        self.take_claim(request, now)
    }

    fn release(&self, room: &str, owner: &str, path: &str) -> Result<()> {
        self.do_release(room, owner, path)
    }

    fn release_with(&self, release: &Release, now: i64) -> Result<()> {
        self.do_release_with(release, now)
    }

    fn last_release(&self, room: &str, path: &str) -> Result<Option<ReleaseNote>> {
        self.optional(
            "SELECT owner,note,ts FROM claim_notes WHERE room=?1 AND path=?2",
            params![room, path],
            |r| {
                Ok(ReleaseNote {
                    owner: r.get(0)?,
                    note: r.get(1)?,
                    ts: r.get(2)?,
                })
            },
        )
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

    fn enqueue_review(&self, item: &NewReview, now: i64) -> Result<i64> {
        self.enqueue_review_row(item, now)
    }

    fn pending_reviews(&self, limit: usize) -> Result<Vec<QueuedReview>> {
        self.pending_review_rows(limit)
    }

    fn set_review_text(&self, id: i64, text: &str) -> Result<()> {
        self.set_review_text_row(id, text)
    }

    fn reviews(&self, limit: usize) -> Result<Vec<QueuedReview>> {
        self.review_rows("1=1 ORDER BY id DESC", limit)
    }

    fn unapplied_reviews(&self, limit: usize) -> Result<Vec<QueuedReview>> {
        self.review_rows(
            "to_room<>'' AND verdict IN ('allow','redact') AND applied='' ORDER BY id",
            limit,
        )
    }

    fn mark_review_applied(&self, id: i64, outcome: &str) -> Result<()> {
        self.mark_review_applied_row(id, outcome)
    }

    fn resolve_review(&self, id: i64, verdict: &str, detail: &str, now: i64) -> Result<bool> {
        self.resolve_review_row(id, verdict, detail, now)
    }

    fn record_audit(&self, entry: &NewAudit, now: i64) -> Result<()> {
        let NewAudit {
            action,
            target,
            detail,
        } = *entry;
        self.exec(
            "INSERT INTO admin_audit(ts,action,target,detail) VALUES(?1,?2,?3,?4)",
            params![now, action, target, detail],
        )
        .map(|_| ())
    }

    fn audit_log(&self, limit: usize) -> Result<Vec<AuditRecord>> {
        self.rows(
            "SELECT ts,action,target,detail FROM admin_audit ORDER BY id DESC LIMIT ?1",
            params![limit as i64],
            |r| {
                Ok(AuditRecord {
                    ts: r.get(0)?,
                    action: r.get(1)?,
                    target: r.get(2)?,
                    detail: r.get(3)?,
                })
            },
        )
    }

    fn policy_get(&self, key: &str) -> Result<Option<String>> {
        self.optional("SELECT value FROM policy WHERE key=?1", params![key], |r| {
            r.get(0)
        })
    }

    fn policy_set(&self, key: &str, value: Option<&str>, now: i64) -> Result<()> {
        match value {
            Some(v) => self.write(
                "INSERT INTO policy(key,value,ts) VALUES(?1,?2,?3)
                 ON CONFLICT(key) DO UPDATE SET value=?2, ts=?3",
                params![key, v, now],
            ),
            None => self.write("DELETE FROM policy WHERE key=?1", params![key]),
        }
    }

    fn policy_list(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        self.rows(
            "SELECT key,value FROM policy WHERE substr(key,1,length(?1))=?1 ORDER BY key",
            params![prefix],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    }

    fn record_refusal(&self, entry: &NewRefusal, now: i64) -> Result<()> {
        let NewRefusal {
            member,
            room,
            operation,
            rule,
        } = *entry;
        self.write(
            "INSERT INTO refusals(ts,member,room,operation,rule) VALUES(?1,?2,?3,?4,?5)",
            params![now, member, room, operation, rule],
        )
    }

    fn refusals(&self, limit: usize) -> Result<Vec<RefusalRecord>> {
        self.rows(
            "SELECT ts,member,room,operation,rule FROM refusals ORDER BY id DESC LIMIT ?1",
            params![limit as i64],
            |r| {
                Ok(RefusalRecord {
                    ts: r.get(0)?,
                    member: r.get(1)?,
                    room: r.get(2)?,
                    operation: r.get(3)?,
                    rule: r.get(4)?,
                })
            },
        )
    }

    fn posts_since(&self, member: &str, since: i64) -> Result<Vec<i64>> {
        self.rows(
            "SELECT ts FROM messages WHERE sender=?1 AND ts>=?2 ORDER BY ts",
            params![member, since],
            |r| r.get(0),
        )
    }

    fn refusal_rules_since(&self, member: &str, since: i64) -> Result<Vec<String>> {
        self.rows(
            "SELECT rule FROM refusals WHERE member=?1 AND ts>=?2 ORDER BY id",
            params![member, since],
            |r| r.get(0),
        )
    }

    fn release_member_claims(&self, member: &str, now: i64) -> Result<usize> {
        self.do_release_member_claims(member, now)
    }

    fn claim_owners(&self) -> Result<Vec<String>> {
        self.claim_owners_all()
    }

    fn retime_member_claims(&self, member: &str, until: i64, extend: bool) -> Result<usize> {
        self.retime_claims(member, until, extend)
    }

    fn claims(&self, room: &str, now: i64) -> Result<Vec<Claim>> {
        self.rows("SELECT path,owner,reason,expires,scope FROM claims WHERE room=?1 AND expires>?2 ORDER BY path", params![room, now], |r| {
            Ok(Claim {
                path: r.get(0)?,
                owner: r.get(1)?,
                reason: r.get(2)?,
                expires: r.get(3)?,
                scope: r.get(4)?,
            })
        })
    }

    fn claim_ages(
        &self,
        room: &str,
        owner: &str,
        now: i64,
    ) -> Result<Vec<crate::domain::AgedClaim>> {
        self.aged_claims(room, owner, now)
    }

    fn deadlock_notices(
        &self,
        room: &str,
        member: &str,
        now: i64,
    ) -> Result<Vec<crate::domain::CycleNotice>> {
        self.cycle_notices(room, member, now)
    }

    fn claim_changes(&self, room: &str, member: &str, now: i64) -> Result<ClaimChanges> {
        self.do_claim_changes(room, member, now)
    }

    fn rooms_of(&self, member: &str) -> Result<Vec<String>> {
        self.rows(
            "SELECT room FROM members WHERE name=?1 ORDER BY room",
            params![member],
            |r| r.get(0),
        )
    }

    fn tail(&self, room: &str, after_seq: i64, limit: usize) -> Result<Vec<Message>> {
        self.rows(&format!(
                "SELECT {MESSAGE_COLUMNS} FROM messages WHERE room=?1 AND seq>?2 ORDER BY seq LIMIT ?3"
            ), params![room, after_seq, limit as i64], |r| self.open_message(r))
    }

    fn head(&self, room: &str) -> Result<i64> {
        self.conn
            .query_row(
                "SELECT COALESCE(MAX(seq),0) FROM messages WHERE room=?1",
                params![room],
                |r| r.get(0),
            )
            .map_err(be)
    }

    fn advance_cursor(&self, at: Seat, seq: i64, now: i64) -> Result<()> {
        let Seat { room, member } = at;
        // One statement, so it is atomic: forward only, and clamped to the room's head so an
        // acknowledgement can never skip a message that has not arrived yet.
        self.exec(
                "INSERT INTO members(room,name,joined,last_seen,last_seq)
                 VALUES(?1,?2,?4,?4,MAX(0,MIN(?3,(SELECT COALESCE(MAX(seq),0) FROM messages WHERE room=?1))))
                 ON CONFLICT(room,name) DO UPDATE SET
                   last_seq=MAX(last_seq,MIN(?3,(SELECT COALESCE(MAX(seq),0) FROM messages WHERE room=?1))),
                   last_seen=?4",
                params![room, member, seq, now],
            )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
