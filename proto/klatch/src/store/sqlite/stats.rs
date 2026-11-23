//! The counts-only usage report (#1377), read from the store without opening it for writing.
//!
//! Every query here selects counts, timestamps, room and member names, or a word from a closed vocabulary. The one
//! place a message body is selected is the ask fold, which takes at most the first 48 characters of the messages whose
//! body starts with an ask word (`seeking:`, `answer to #N:`, `closed #N`) to read the number and the sender; that text
//! is matched and dropped, never stored in the report. A message under a sealed label is never read at all.

mod asks;
mod events;
mod members;
mod rooms;

use super::SqliteStore;
use super::connect::be;
use super::schema::{WAIT_WINDOW_SECS, user_version};
use crate::domain::{
    ClaimCounts, Counts, Result, StatsOptions, UsageStats, claim_class, day_of, vocab,
};
use crate::keys::Key;
use rusqlite::types::Value;
use rusqlite::{Connection, Params, params_from_iter};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// Whether a count query over a catalogue finds anything.
fn present(conn: &Connection, sql: &str, args: impl Params) -> Result<bool> {
    let n: i64 = conn.query_row(sql, args, |r| r.get(0)).map_err(be)?;
    Ok(n > 0)
}

/// Whether `table` has `column`: a store read without migrating may predate it.
fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    present(
        conn,
        "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name=?2",
        [table, column],
    )
}

fn has(conn: &Connection, table: &str) -> Result<bool> {
    present(
        conn,
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        [table],
    )
}

/// Binds `?1` (the room, or null for all) and `?2` (the first second) to the placeholders `sql` uses.
fn binds(sql: &str, o: &StatsOptions) -> Vec<Value> {
    let room = o.room.clone().map_or(Value::Null, Value::Text);
    let since = Value::Integer(o.since.unwrap_or(0));
    match (sql.contains("?1"), sql.contains("?2")) {
        (_, true) => vec![room, since],
        (true, false) => vec![room],
        _ => vec![],
    }
}

fn rows<T>(
    conn: &Connection,
    sql: &str,
    o: &StatsOptions,
    f: impl Fn(&rusqlite::Row) -> rusqlite::Result<T>,
) -> Result<Vec<T>> {
    let mut stmt = conn.prepare(sql).map_err(be)?;
    let it = stmt
        .query_map(params_from_iter(binds(sql, o)), f)
        .map_err(be)?;
    it.collect::<std::result::Result<_, _>>().map_err(be)
}

fn one(conn: &Connection, sql: &str, o: &StatsOptions) -> Result<i64> {
    Ok(rows(conn, sql, o, |r| r.get::<_, i64>(0))?
        .first()
        .copied()
        .unwrap_or(0))
}

/// `(name, count)` rows from a query that selects a string and a count, unchanged.
fn name_counts(conn: &Connection, sql: &str, o: &StatsOptions) -> Result<Counts> {
    rows(conn, sql, o, |r| Ok((r.get(0)?, r.get(1)?)))
}

/// `name_counts` for member names: control characters are dropped so a name cannot move the cursor of a terminal.
fn printable_counts(conn: &Connection, sql: &str, o: &StatsOptions) -> Result<Counts> {
    Ok(name_counts(conn, sql, o)?
        .into_iter()
        .map(|(n, c)| (n.chars().filter(|c| !c.is_control()).collect(), c))
        .collect())
}

/// `(word, count)` pairs from a query that selects a nullable word and a count; the word passes `vocab`.
fn words(conn: &Connection, table: &str, sql: &str, o: &StatsOptions) -> Result<Counts> {
    if !has(conn, table)? {
        return Ok(Vec::new());
    }
    Ok(rows(conn, sql, o, |r| {
        Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?))
    })?
    .into_iter()
    .map(|(w, n)| (w.map_or_else(|| "(none)".to_string(), |w| vocab(&w)), n))
    .collect())
}

/// Counts per claim class, in a fixed order, classes with none left out.
fn by_class(paths: impl Iterator<Item = String>) -> Counts {
    let mut n: BTreeMap<&str, i64> = BTreeMap::new();
    for p in paths {
        *n.entry(claim_class(&p)).or_default() += 1;
    }
    ["file", "ticket", "ask", "other"]
        .iter()
        .filter_map(|c| n.get(c).map(|n| (c.to_string(), *n)))
        .collect()
}

/// The claim paths a query selects from `table`, counted by class; nothing when the table is missing.
fn class_counts(conn: &Connection, table: &str, sql: &str, o: &StatsOptions) -> Result<Counts> {
    if !has(conn, table)? {
        return Ok(Vec::new());
    }
    let paths = rows(conn, sql, o, |r| r.get::<_, String>(0))?;
    Ok(by_class(paths.into_iter()))
}

const LIVE: &str = "kind IS NOT 'archived' AND (?1 IS NULL OR room=?1) AND ts>=?2";

/// The labels under which messages are sealed: counted, never read.
fn sealed_labels(conn: &Connection, o: &StatsOptions) -> Result<HashSet<String>> {
    if !has(conn, "policy")? {
        return Ok(HashSet::new());
    }
    let sql = "SELECT substr(key,8) FROM policy WHERE key LIKE 'sealed:%'";
    Ok(rows(conn, sql, o, |r| r.get::<_, String>(0))?
        .into_iter()
        .collect())
}

/// Messages by UTC day, and by room and day, oldest first.
fn fill_days(conn: &Connection, o: &StatsOptions, s: &mut UsageStats) -> Result<()> {
    let by_day = rows(
        conn,
        &format!(
            "SELECT ts/86400,COUNT(*) FROM messages WHERE {LIVE} GROUP BY ts/86400 ORDER BY 1"
        ),
        o,
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
    )?;
    s.days = by_day
        .into_iter()
        .map(|(d, n)| (day_of(d * 86_400), n))
        .collect();
    let by_room_day = rows(
        conn,
        &format!(
            "SELECT room,ts/86400,COUNT(*) FROM messages WHERE {LIVE} GROUP BY room,ts/86400 ORDER BY 1,2"
        ),
        o,
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        },
    )?;
    s.room_days = by_room_day
        .into_iter()
        .map(|(room, d, n)| (room, day_of(d * 86_400), n))
        .collect();
    Ok(())
}

/// Senders, attribution and message kinds over the live messages.
fn fill_senders(conn: &Connection, o: &StatsOptions, s: &mut UsageStats) -> Result<()> {
    s.senders_distinct = one(
        conn,
        &format!("SELECT COUNT(DISTINCT sender) FROM messages WHERE {LIVE}"),
        o,
    )?;
    s.verified_senders = one(
        conn,
        &format!(
            "SELECT COUNT(DISTINCT sender) FROM messages WHERE {LIVE} AND attribution='verified'"
        ),
        o,
    )?;
    s.senders_top = printable_counts(
        conn,
        &format!(
            "SELECT sender,COUNT(*) c FROM messages WHERE {LIVE} AND sender IS NOT NULL GROUP BY sender ORDER BY c DESC, sender LIMIT {}",
            o.top
        ),
        o,
    )?;
    s.attribution = words(
        conn,
        "messages",
        &format!(
            "SELECT attribution,COUNT(*) c FROM messages WHERE {LIVE} GROUP BY attribution ORDER BY c DESC, 1"
        ),
        o,
    )?;
    s.kinds = words(
        conn,
        "messages",
        &format!(
            "SELECT kind,COUNT(*) c FROM messages WHERE {LIVE} GROUP BY kind ORDER BY c DESC, 1"
        ),
        o,
    )?;
    Ok(())
}

/// Live claims (not yet expired at `now`), notes, members waiting, yields and the event record.
fn claim_counts(conn: &Connection, o: &StatsOptions) -> Result<ClaimCounts> {
    let live_at = StatsOptions {
        since: Some(o.now),
        ..o.clone()
    };
    let waiting_since = StatsOptions {
        since: Some(o.now - WAIT_WINDOW_SECS),
        ..o.clone()
    };
    let yields = if has(conn, "claim_yields")? {
        one(
            conn,
            "SELECT COUNT(*) FROM claim_yields WHERE (?1 IS NULL OR room=?1) AND ts>=?2",
            o,
        )?
    } else {
        0
    };
    Ok(ClaimCounts {
        live: class_counts(
            conn,
            "claims",
            "SELECT path FROM claims WHERE (?1 IS NULL OR room=?1) AND expires>?2",
            &live_at,
        )?,
        notes: class_counts(
            conn,
            "claim_notes",
            "SELECT path FROM claim_notes WHERE (?1 IS NULL OR room=?1) AND ts>=?2",
            o,
        )?,
        waiting: class_counts(
            conn,
            "claim_waits",
            "SELECT path FROM claim_waits WHERE (?1 IS NULL OR room=?1) AND ts>=?2",
            &waiting_since,
        )?,
        yields,
        history: events::claim_history(conn, o)?,
    })
}

/// A per-event table over `hook_runs`: `agg` summed or maxed per event, events with none left out when `having`.
fn hook_words(conn: &Connection, o: &StatsOptions, agg: &str, having: &str) -> Result<Counts> {
    words(
        conn,
        "hook_runs",
        &format!(
            "SELECT event,{agg} c FROM hook_runs WHERE ?1 IS NOT 0 AND last_ts>=?2 GROUP BY event{having} ORDER BY c DESC, 1"
        ),
        o,
    )
}

/// Hook runs by event and, for a store with the byte columns, the bytes the hooks put into sessions.
fn fill_hooks(conn: &Connection, o: &StatsOptions, s: &mut UsageStats) -> Result<()> {
    s.hooks = hook_words(conn, o, "SUM(runs)", "")?;
    if !has_column(conn, "hook_runs", "total_bytes")? {
        return Ok(());
    }
    s.hook_bytes = hook_words(conn, o, "SUM(total_bytes)", " HAVING c>0")?;
    s.hook_bytes_max = hook_words(conn, o, "MAX(max_bytes)", " HAVING c>0")?;
    s.sessions_top_bytes = printable_counts(
        conn,
        &format!(
            "SELECT member,SUM(total_bytes) c FROM hook_runs WHERE ?1 IS NOT 0 AND last_ts>=?2 GROUP BY member HAVING c>0 ORDER BY c DESC, member LIMIT {}",
            o.top
        ),
        o,
    )?;
    Ok(())
}

/// Worker credentials issued in the window, and how many of those were revoked.
fn fill_credentials(conn: &Connection, o: &StatsOptions, s: &mut UsageStats) -> Result<()> {
    if !has(conn, "credentials")? {
        return Ok(());
    }
    s.credentials_issued = one(
        conn,
        "SELECT COUNT(*) FROM credentials WHERE ?1 IS NOT 0 AND issued>=?2",
        o,
    )?;
    s.credentials_revoked = one(
        conn,
        "SELECT COUNT(*) FROM credentials WHERE ?1 IS NOT 0 AND issued>=?2 AND revoked<>0",
        o,
    )?;
    Ok(())
}

fn collect(conn: &Connection, o: &StatsOptions) -> Result<UsageStats> {
    let sealed = sealed_labels(conn, o)?;
    let mut s = UsageStats {
        schema: user_version(conn)?,
        room: o.room.clone(),
        since: o.since,
        now: o.now,
        ..UsageStats::default()
    };
    s.rooms = rooms::rooms(conn, o, &sealed)?;
    fill_days(conn, o, &mut s)?;
    fill_senders(conn, o, &mut s)?;
    s.asks = asks::asks(conn, o, &sealed)?;
    s.claims = claim_counts(conn, o)?;
    s.machine_checkpoints = events::machine_checkpoint_counts(conn, o)?;
    s.machine_status = events::machine_status_counts(conn, o)?;
    s.passive_claims = events::passive_claim_counts(conn, o)?;
    s.ask_routing = events::ask_routing_counts(conn, o)?;
    s.refusals = words(
        conn,
        "refusals",
        "SELECT rule,COUNT(*) c FROM refusals WHERE (?1 IS NULL OR room=?1) AND ts>=?2 GROUP BY rule ORDER BY c DESC, 1",
        o,
    )?;
    s.members = members::members(conn, o)?;
    fill_hooks(conn, o, &mut s)?;
    s.audit = words(
        conn,
        "admin_audit",
        "SELECT action,COUNT(*) c FROM admin_audit WHERE ?1 IS NOT 0 AND ts>=?2 GROUP BY action ORDER BY c DESC, 1",
        o,
    )?;
    fill_credentials(conn, o, &mut s)?;
    Ok(s)
}

impl SqliteStore {
    /// The report over this open store.
    pub fn usage_stats(&self, opts: &StatsOptions) -> Result<UsageStats> {
        collect(&self.conn, opts)
    }

    /// The report over a store file, opened read-only: no migration, no journal switch, nothing created.
    pub fn read_usage_stats(path: &Path, key: &Key, opts: &StatsOptions) -> Result<UsageStats> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(be)?;
        Self::unlock(&conn, key)?;
        collect(&conn, opts)
    }
}
