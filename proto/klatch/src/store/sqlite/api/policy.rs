//! The SQLite side of the `PolicyStore` area.

use super::super::SqliteStore;
use crate::domain::{AuditRecord, Error, RefusalRecord, Result};
use crate::store::Census;
use crate::store::PolicyStore;
use crate::store::area::{is_proof_key, normalize_key};
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

    fn mark_label_sealed_converting(&self, label: &str, now: i64) -> Result<usize> {
        self.mark_sealed_converting(label, now)
    }

    fn audit_log(&self, limit: usize) -> Result<Vec<AuditRecord>> {
        self.rows(
            "SELECT ts,action,target,detail FROM admin_audit ORDER BY id DESC LIMIT ?1",
            params![limit as i64],
            audit_from_row,
        )
    }

    fn audit_page(&self, limit: usize, only: Option<&str>) -> Result<crate::store::AuditPage> {
        use crate::store::FORWARDED_IGNORED_ACTION as NOTE;
        let rows = match only {
            Some(action) => self.rows(
                "SELECT ts,action,target,detail FROM admin_audit WHERE action=?2 ORDER BY id DESC LIMIT ?1",
                params![limit as i64, action],
                audit_from_row,
            )?,
            None => self.rows(
                "SELECT ts,action,target,detail FROM admin_audit WHERE action<>?2 ORDER BY id DESC LIMIT ?1",
                params![limit as i64, NOTE],
                audit_from_row,
            )?,
        };
        let hidden = match only {
            Some(_) => 0,
            None => self
                .optional(
                    "SELECT COUNT(*) FROM admin_audit WHERE action=?1",
                    params![NOTE],
                    |r| r.get::<_, i64>(0),
                )?
                .unwrap_or(0)
                .max(0) as u64,
        };
        Ok(crate::store::AuditPage { rows, hidden })
    }

    fn last_audit_ts(&self, action: &str, target: &str, since: i64) -> Result<Option<i64>> {
        self.optional(
            "SELECT MAX(ts) FROM admin_audit WHERE action=?1 AND target=?2 AND ts>=?3",
            params![action, target, since],
            |r| r.get::<_, Option<i64>>(0),
        )
        .map(Option::flatten)
    }

    fn audit_census(&self, q: &Census) -> Result<(u64, u64)> {
        if q.targets.is_empty() || q.cap == 0 {
            return Ok((0, 0));
        }
        self.optional(
            &census_sql(q.targets.len()),
            rusqlite::params_from_iter(census_params(q)),
            census_from_row,
        )
        .map(|c| c.unwrap_or((0, 0)))
    }

    fn policy_get(&self, key: &str) -> Result<Option<String>> {
        self.optional("SELECT value FROM policy WHERE key=?1", params![key], |r| {
            r.get(0)
        })
    }

    fn policy_set(&self, key: &str, value: Option<&str>, now: i64) -> Result<()> {
        if is_proof_key(key) {
            // key name only, never the value; the audit row is kept even though the write is refused
            // at most one row a minute per key and a few per family of other keys, counting rows still queued in an
            // open transaction, so a loop of refused writes cannot fill the audit or the queue (#1588, #1701); what
            // was left out is one summary row for the minute (#1709)
            let nk = normalize_key(key);
            if self.refusal_throttled(&nk, now) {
                self.note_throttled(now);
            } else {
                self.audit_refusal("policy-reserved-refused", &nk, now);
            }
            return Err(Error::Refused {
                rule: "admin-passphrase-rows".into(),
                reason: format!(
                    "the policy key {key} belongs to the admin passphrase and is changed only by `klatch admin` passphrase commands"
                ),
            });
        }
        self.write_policy(key, value, now)
    }

    fn reserved_policy_set(
        &self,
        _cap: &crate::proof::ProofCap,
        key: &str,
        value: Option<&str>,
        now: i64,
    ) -> Result<()> {
        if !is_proof_key(key) {
            return Err(Error::Invalid(format!(
                "{key} is not a key of the admin passphrase"
            )));
        }
        self.write_policy(key, value, now)
    }

    fn atomically(&self, act: &mut dyn FnMut() -> Result<()>) -> Result<()> {
        self.immediate(act)
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

impl SqliteStore {
    fn write_policy(&self, key: &str, value: Option<&str>, now: i64) -> Result<()> {
        match value {
            Some(v) => self.write(
                "INSERT INTO policy(key,value,ts) VALUES(?1,?2,?3)
                 ON CONFLICT(key) DO UPDATE SET value=?2, ts=?3",
                params![key, v, now],
            ),
            None => self.write("DELETE FROM policy WHERE key=?1", params![key]),
        }
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

/// The census query (#1310) for `n` targets: `?1` the action, `?2` the first moment that counts, `?3` the cap, `?4`
/// the last moment that counts, `?5` and on the targets. Each target is a range of the `(action, target, ts)` index read for at most `?3` rows, and the
/// union is cut at `?3` too, so no more than `n * cap` rows are touched however large the audit is. The outer query
/// counts them and the distinct `terminal=` labels among them.
pub(in crate::store::sqlite) fn census_sql(n: usize) -> String {
    let each: Vec<String> = (0..n)
        .map(|i| {
            format!(
                "SELECT detail FROM (SELECT detail FROM admin_audit \
                 WHERE action=?1 AND target=?{} AND ts>=?2 AND ts<=?4 LIMIT ?3)",
                5 + i
            )
        })
        .collect();
    format!(
        "SELECT COUNT(*), COUNT(DISTINCT CASE WHEN instr(detail,'terminal=')>0 \
         THEN substr(detail, instr(detail,'terminal=')+9) END) \
         FROM ({} LIMIT ?3)",
        each.join(" UNION ALL ")
    )
}

pub(in crate::store::sqlite) fn census_params(q: &Census) -> Vec<rusqlite::types::Value> {
    use rusqlite::types::Value;
    let mut p = vec![
        Value::Text(q.action.to_string()),
        Value::Integer(q.since),
        Value::Integer(q.cap.min(i64::MAX as u64) as i64),
        Value::Integer(q.until),
    ];
    p.extend(q.targets.iter().map(|t| Value::Text((*t).to_string())));
    p
}

pub(in crate::store::sqlite) fn census_from_row(
    r: &rusqlite::Row<'_>,
) -> rusqlite::Result<(u64, u64)> {
    Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64))
}

/// Rows of refused writes to one family of admin passphrase keys that one minute may hold (#1701). The keys the
/// proof reads (`is_real_key`) are not counted in it and keep their own budget.
const FAMILY_CAP: u64 = 8;

/// The keys the proof module really reads. Each has its own one-row-a-minute budget outside the family cap, so a
/// flood of junk keys cannot hide a refusal of one of them (#1709).
fn is_real_key(key: &str) -> bool {
    [
        crate::proof::VERIFIER_KEY,
        crate::proof::FAILS_KEY,
        crate::proof::WINDOW_KEY,
    ]
    .contains(&key)
}

/// The family of a reserved key: `admin-unlock-*` is one, everything else under `admin-passphrase` the other.
fn family(key: &str) -> &'static str {
    if key.starts_with("admin-unlock-") {
        "admin-unlock-"
    } else {
        "admin-passphrase"
    }
}

impl SqliteStore {
    /// Whether a refusal of `key` at `now` is left unaudited: the key already has a row in the last minute, or it is
    /// not a real key and its family already has `FAMILY_CAP` rows of other keys. Written rows and rows queued in an
    /// open transaction both count; a row stamped after `now` (the clock stepped back) counts for neither, or it
    /// would silence the audit (#1701).
    fn refusal_throttled(&self, key: &str, now: i64) -> bool {
        const ACTION: &str = "policy-reserved-refused";
        let fam = family(key);
        let (mut same, mut in_family) = self
            .optional(
                "SELECT COALESCE(SUM(target=?3),0), COALESCE(SUM(target NOT IN (?6,?7,?8)),0) FROM admin_audit \
                 WHERE action=?1 AND ts BETWEEN ?2 AND ?4 AND substr(target,1,length(?5))=?5",
                params![
                    ACTION,
                    now - 60,
                    key,
                    now,
                    fam,
                    crate::proof::VERIFIER_KEY,
                    crate::proof::FAILS_KEY,
                    crate::proof::WINDOW_KEY
                ],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )
            .ok()
            .flatten()
            .unwrap_or((0, 0));
        for (action, target, ts, _) in self.deferred_audit.borrow().iter() {
            if *action == ACTION && (now - 60..=now).contains(ts) && target.starts_with(fam) {
                in_family += i64::from(!is_real_key(target));
                same += i64::from(target == key);
            }
        }
        same > 0 || (!is_real_key(key) && in_family as u64 >= FAMILY_CAP)
    }
}
