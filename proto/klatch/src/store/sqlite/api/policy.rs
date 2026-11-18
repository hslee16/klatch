//! The SQLite side of the `PolicyStore` area.

use super::super::SqliteStore;
use crate::domain::{AuditRecord, RefusalRecord, Result};
use crate::store::PolicyStore;
use crate::store::{NewAudit, NewRefusal};
use rusqlite::{Row, params};

impl PolicyStore for SqliteStore {
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
            audit_from_row,
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
            refusal_from_row,
        )
    }

    fn refusals_of(&self, member: &str, limit: usize) -> Result<Vec<RefusalRecord>> {
        self.rows(
            "SELECT ts,member,room,operation,rule FROM refusals WHERE member=?1 ORDER BY id DESC LIMIT ?2",
            params![member, limit as i64],
            refusal_from_row,
        )
    }

    fn refusal_rules_since(&self, member: &str, since: i64) -> Result<Vec<String>> {
        self.rows(
            "SELECT rule FROM refusals WHERE member=?1 AND ts>=?2 ORDER BY id",
            params![member, since],
            |r| r.get(0),
        )
    }
}

fn audit_from_row(r: &Row<'_>) -> rusqlite::Result<AuditRecord> {
    Ok(AuditRecord {
        ts: r.get(0)?,
        action: r.get(1)?,
        target: r.get(2)?,
        detail: r.get(3)?,
    })
}

fn refusal_from_row(r: &Row<'_>) -> rusqlite::Result<RefusalRecord> {
    Ok(RefusalRecord {
        ts: r.get(0)?,
        member: r.get(1)?,
        room: r.get(2)?,
        operation: r.get(3)?,
        rule: r.get(4)?,
    })
}
