//! The counts-only usage report (#1377), read from the store without opening it for writing.
//!
//! Every query here selects counts, timestamps, room and member names, or a word from a closed vocabulary. The one
//! place a message body is selected is the ask fold, which takes at most the first 48 characters of the messages whose
//! body starts with an ask word (`seeking:`, `answer to #N:`, `closed #N`) to read the number and the sender; that text
//! is matched and dropped, never stored in the report. A message under a sealed label is never read at all.

use super::SqliteStore;
use super::connect::be;
use super::schema::{WAIT_WINDOW_SECS, user_version};
use crate::domain::{
    AskCounts, ClaimCounts, ClaimHistory, Counts, MemberCounts, Result, RoomClass, RoomCount,
    StatsOptions, UsageStats, answer_target, claim_class, closed_target, day_of, is_ask,
    median_and_p90, vocab,
};
use crate::keys::Key;
use rusqlite::types::Value;
use rusqlite::{Connection, params_from_iter};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

/// The characters of a body the ask fold may read.
const ASK_PREFIX_CHARS: usize = 48;

/// Whether `table` has `column`: a store read without migrating may predate it.
fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name=?2",
            [table, column],
            |r| r.get(0),
        )
        .map_err(be)?;
    Ok(n > 0)
}

fn has(conn: &Connection, table: &str) -> Result<bool> {
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |r| r.get(0),
        )
        .map_err(be)?;
    Ok(n > 0)
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

fn paths(conn: &Connection, table: &str, sql: &str, o: &StatsOptions) -> Result<Vec<String>> {
    if !has(conn, table)? {
        return Ok(Vec::new());
    }
    rows(conn, sql, o, |r| r.get::<_, String>(0))
}

const LIVE: &str = "kind IS NOT 'archived' AND (?1 IS NULL OR room=?1) AND ts>=?2";

fn rooms(conn: &Connection, o: &StatsOptions, sealed: &HashSet<String>) -> Result<Vec<RoomCount>> {
    let labels: HashMap<String, String> = if has(conn, "policy")? {
        rows(
            conn,
            "SELECT substr(key,7), value FROM policy WHERE key LIKE 'label:%'",
            o,
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?
        .into_iter()
        .collect()
    } else {
        HashMap::new()
    };
    let lowest = if has(conn, "policy")? {
        rows(conn, "SELECT value FROM policy WHERE key='chain'", o, |r| {
            r.get::<_, String>(0)
        })?
        .first()
        .and_then(|c| c.split(',').next().map(|s| s.trim().to_string()))
    } else {
        None
    }
    .unwrap_or_else(|| "public".into());
    let mut by_room: BTreeMap<String, RoomCount> = BTreeMap::new();
    let groups = rows(
        conn,
        &format!(
            "SELECT room,label,COUNT(*),MIN(ts),MAX(ts) FROM messages WHERE {LIVE} GROUP BY room,label"
        ),
        o,
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        },
    )?;
    for (room, label, n, first, last) in groups {
        let class = if room.starts_with(crate::domain::DM_PREFIX) {
            RoomClass::Dm
        } else if labels.get(&room).is_some_and(|l| *l != lowest) {
            RoomClass::Private
        } else {
            RoomClass::Ordinary
        };
        let e = by_room.entry(room.clone()).or_insert(RoomCount {
            room,
            class,
            messages: 0,
            sealed: 0,
            senders: 0,
            first_ts: first,
            last_ts: last,
        });
        e.messages += n;
        if sealed.contains(&label) {
            e.sealed += n;
        }
        e.first_ts = e.first_ts.min(first);
        e.last_ts = e.last_ts.max(last);
    }
    for (room, n) in rows(
        conn,
        &format!("SELECT room,COUNT(DISTINCT sender) FROM messages WHERE {LIVE} GROUP BY room"),
        o,
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
    )? {
        if let Some(e) = by_room.get_mut(&room) {
            e.senders = n;
        }
    }
    let mut out: Vec<RoomCount> = by_room.into_values().collect();
    out.sort_by(|a, b| b.messages.cmp(&a.messages).then(a.room.cmp(&b.room)));
    Ok(out)
}

/// The asks, their answers and closes, from the first characters of ask-word messages outside sealed labels.
fn asks(conn: &Connection, o: &StatsOptions, sealed: &HashSet<String>) -> Result<AskCounts> {
    let mut out = AskCounts {
        sealed_unread: one(
            conn,
            &format!(
                "SELECT COUNT(*) FROM messages WHERE {LIVE} AND label IN (SELECT substr(key,8) FROM policy WHERE key LIKE 'sealed:%')"
            ),
            o,
        )?,
        ..AskCounts::default()
    };
    let words = rows(
        conn,
        &format!(
            "SELECT room,seq,sender,ts,label,substr(body,1,{ASK_PREFIX_CHARS}) FROM messages \
             WHERE kind IS NOT 'archived' AND (?1 IS NULL OR room=?1) AND ?2>=0 \
             AND (body LIKE 'seeking:%' OR body LIKE 'answer to #%' OR body LIKE 'closed #%') ORDER BY room,seq"
        ),
        o,
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?.unwrap_or_default(),
            ))
        },
    )?;
    let since = o.since.unwrap_or(0);
    // (room, seq) -> (asker, ts) for the asks created in the window
    let mut created: HashMap<(String, i64), (String, i64)> = HashMap::new();
    for (room, seq, sender, ts, label, text) in &words {
        if !sealed.contains(label) && is_ask(text) && *ts >= since {
            created.insert((room.clone(), *seq), (sender.clone(), *ts));
        }
    }
    let mut first: HashMap<(String, i64), i64> = HashMap::new();
    let mut closed: HashSet<(String, i64)> = HashSet::new();
    for (room, seq, sender, ts, label, text) in &words {
        if sealed.contains(label) {
            continue;
        }
        if let Some(n) = answer_target(text)
            && let Some((asker, at)) = created.get(&(room.clone(), n))
            && sender != asker
            && *seq > n
        {
            let e = first.entry((room.clone(), n)).or_insert(i64::MAX);
            *e = (*e).min((*ts - *at).max(0));
        }
        if let Some(n) = closed_target(text)
            && created.contains_key(&(room.clone(), n))
        {
            closed.insert((room.clone(), n));
        }
    }
    out.created = created.len() as i64;
    out.answered = first.len() as i64;
    out.closed = closed.len() as i64;
    (out.first_answer_median, out.first_answer_p90) =
        median_and_p90(first.values().copied().collect());
    if has(conn, "ask_scopes")? {
        let scoped = rows(
            conn,
            "SELECT room,seq FROM ask_scopes WHERE (?1 IS NULL OR room=?1)",
            o,
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )?;
        out.with_scope = scoped.iter().filter(|k| created.contains_key(*k)).count() as i64;
    }
    if has(conn, "ask_refs")? {
        let refs = rows(
            conn,
            "SELECT room,seq,kind FROM ask_refs WHERE (?1 IS NULL OR room=?1)",
            o,
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )?;
        let mut with: HashSet<(String, i64)> = HashSet::new();
        let mut kinds: BTreeMap<String, i64> = BTreeMap::new();
        for (room, seq, kind) in refs {
            if created.contains_key(&(room.clone(), seq)) {
                with.insert((room, seq));
                *kinds.entry(vocab(&kind)).or_default() += 1;
            }
        }
        out.with_refs = with.len() as i64;
        out.ref_kinds = kinds.into_iter().collect();
    }
    Ok(out)
}

fn members(conn: &Connection, o: &StatsOptions) -> Result<MemberCounts> {
    if !has(conn, "members")? {
        return Ok(MemberCounts::default());
    }
    let pop = "(?1 IS NULL OR room=?1) AND joined>=?2";
    let parents = if has(conn, "member_parent")? {
        one(
            conn,
            &format!(
                "SELECT COUNT(DISTINCT name) FROM members WHERE {pop} AND name IN (SELECT member FROM member_parent)"
            ),
            o,
        )?
    } else {
        0
    };
    // One row per posting name: the furthest any of its rows read, was shown, and the day of its first post (#1411).
    let posters = rows(
        conn,
        &format!(
            "SELECT MAX(COALESCE(last_seq,0)), MAX(MAX(hook_seq,wait_seq,wake_seq)), \
             (SELECT MIN(ts) FROM messages m WHERE m.sender=members.name AND m.kind IS NOT 'archived' AND (?1 IS NULL OR m.room=?1)) AS first \
             FROM members WHERE {pop} GROUP BY name HAVING first IS NOT NULL"
        ),
        o,
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        },
    )?;
    let mut by_day: BTreeMap<String, i64> = BTreeMap::new();
    for (read, _, first) in &posters {
        if *read == 0 {
            *by_day.entry(day_of(*first)).or_default() += 1;
        }
    }
    Ok(MemberCounts {
        posted: i64::try_from(posters.len()).unwrap_or(i64::MAX),
        never_read: by_day.values().sum(),
        never_shown: i64::try_from(
            posters
                .iter()
                .filter(|(read, shown, _)| *read == 0 && *shown == 0)
                .count(),
        )
        .unwrap_or(i64::MAX),
        never_read_days: by_day.into_iter().collect(),
        joined: one(
            conn,
            &format!("SELECT COUNT(DISTINCT name) FROM members WHERE {pop}"),
            o,
        )?,
        never_posted: one(
            conn,
            &format!(
                "SELECT COUNT(DISTINCT name) FROM members WHERE {pop} AND NOT EXISTS \
                 (SELECT 1 FROM messages m WHERE m.sender=members.name AND m.kind IS NOT 'archived' AND (?1 IS NULL OR m.room=?1))"
            ),
            o,
        )?,
        marked_done: one(
            conn,
            &format!(
                "SELECT COUNT(DISTINCT name) FROM members WHERE {pop} AND status LIKE 'done%'"
            ),
            o,
        )?,
        muted: one(
            conn,
            &format!("SELECT COUNT(DISTINCT name) FROM members WHERE {pop} AND muted<>0"),
            o,
        )?,
        with_parent: parents,
    })
}

/// The claim event record (#1386): counts by class and word from a closed vocabulary, and medians of seconds. Nothing
/// in the table is a name, so nothing here can print one.
fn claim_history(conn: &Connection, o: &StatsOptions) -> Result<ClaimHistory> {
    const T: &str = "claim_events";
    let by = |col: &str, event: &str| {
        words(
            conn,
            T,
            &format!(
                "SELECT {col},COUNT(*) c FROM claim_events WHERE event='{event}' AND (?1 IS NULL OR room=?1) AND ts>=?2 GROUP BY {col} ORDER BY c DESC, 1"
            ),
            o,
        )
    };
    let mut h = ClaimHistory {
        granted: by("class", "granted")?,
        refused: by("reason", "refused")?,
        released: by("class", "released")?,
        released_by: by("reason", "released")?,
        expired: by("class", "expired")?,
        ..ClaimHistory::default()
    };
    if !has(conn, T)? {
        return Ok(h);
    }
    let w = "(?1 IS NULL OR room=?1) AND ts>=?2";
    h.takeovers = one(
        conn,
        &format!("SELECT COUNT(*) FROM claim_events WHERE event='takeover' AND {w}"),
        o,
    )?;
    let waits = rows(
        conn,
        &format!(
            "SELECT waited FROM claim_events WHERE event='granted' AND waited IS NOT NULL AND {w}"
        ),
        o,
        |r| r.get::<_, i64>(0),
    )?;
    h.handled = waits.len() as i64;
    h.handled_wait_median = median_and_p90(waits).0;
    let holds = rows(
        conn,
        &format!(
            "SELECT held FROM claim_events WHERE event IN ('released','expired') AND held>0 AND {w}"
        ),
        o,
        |r| r.get::<_, i64>(0),
    )?;
    h.holds = holds.len() as i64;
    h.hold_median = median_and_p90(holds).0;
    Ok(h)
}

fn collect(conn: &Connection, o: &StatsOptions) -> Result<UsageStats> {
    let sealed: HashSet<String> = if has(conn, "policy")? {
        rows(
            conn,
            "SELECT substr(key,8) FROM policy WHERE key LIKE 'sealed:%'",
            o,
            |r| r.get::<_, String>(0),
        )?
        .into_iter()
        .collect()
    } else {
        HashSet::new()
    };
    let mut s = UsageStats {
        schema: user_version(conn)?,
        room: o.room.clone(),
        since: o.since,
        now: o.now,
        ..UsageStats::default()
    };
    s.rooms = rooms(conn, o, &sealed)?;
    s.days = rows(
        conn,
        &format!(
            "SELECT ts/86400,COUNT(*) FROM messages WHERE {LIVE} GROUP BY ts/86400 ORDER BY 1"
        ),
        o,
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
    )?
    .into_iter()
    .map(|(d, n)| (day_of(d * 86_400), n))
    .collect();
    s.room_days = rows(
        conn,
        &format!("SELECT room,ts/86400,COUNT(*) FROM messages WHERE {LIVE} GROUP BY room,ts/86400 ORDER BY 1,2"),
        o,
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)),
    )?
    .into_iter()
    .map(|(room, d, n)| (room, day_of(d * 86_400), n))
    .collect();
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
    s.senders_top = rows(
        conn,
        &format!("SELECT sender,COUNT(*) c FROM messages WHERE {LIVE} AND sender IS NOT NULL GROUP BY sender ORDER BY c DESC, sender LIMIT {}", o.top),
        o,
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
    )?
    .into_iter()
    .map(|(n, c)| (n.chars().filter(|c| !c.is_control()).collect(), c))
    .collect();
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
    s.asks = asks(conn, o, &sealed)?;
    s.claims = ClaimCounts {
        live: by_class(
            paths(
                conn,
                "claims",
                "SELECT path FROM claims WHERE (?1 IS NULL OR room=?1) AND expires>?2",
                &StatsOptions {
                    since: Some(o.now),
                    ..o.clone()
                },
            )?
            .into_iter(),
        ),
        notes: by_class(
            paths(
                conn,
                "claim_notes",
                "SELECT path FROM claim_notes WHERE (?1 IS NULL OR room=?1) AND ts>=?2",
                o,
            )?
            .into_iter(),
        ),
        waiting: by_class(
            paths(
                conn,
                "claim_waits",
                "SELECT path FROM claim_waits WHERE (?1 IS NULL OR room=?1) AND ts>=?2",
                &StatsOptions {
                    since: Some(o.now - WAIT_WINDOW_SECS),
                    ..o.clone()
                },
            )?
            .into_iter(),
        ),
        yields: if has(conn, "claim_yields")? {
            one(
                conn,
                "SELECT COUNT(*) FROM claim_yields WHERE (?1 IS NULL OR room=?1) AND ts>=?2",
                o,
            )?
        } else {
            0
        },
        history: claim_history(conn, o)?,
    };
    s.refusals = words(
        conn,
        "refusals",
        "SELECT rule,COUNT(*) c FROM refusals WHERE (?1 IS NULL OR room=?1) AND ts>=?2 GROUP BY rule ORDER BY c DESC, 1",
        o,
    )?;
    s.members = members(conn, o)?;
    s.hooks = words(
        conn,
        "hook_runs",
        "SELECT event,SUM(runs) c FROM hook_runs WHERE ?1 IS NOT 0 AND last_ts>=?2 GROUP BY event ORDER BY c DESC, 1",
        o,
    )?;
    if has_column(conn, "hook_runs", "total_bytes")? {
        s.hook_bytes = words(
            conn,
            "hook_runs",
            "SELECT event,SUM(total_bytes) c FROM hook_runs WHERE ?1 IS NOT 0 AND last_ts>=?2 GROUP BY event HAVING c>0 ORDER BY c DESC, 1",
            o,
        )?;
        s.hook_bytes_max = words(
            conn,
            "hook_runs",
            "SELECT event,MAX(max_bytes) c FROM hook_runs WHERE ?1 IS NOT 0 AND last_ts>=?2 GROUP BY event HAVING c>0 ORDER BY c DESC, 1",
            o,
        )?;
        s.sessions_top_bytes = rows(
            conn,
            &format!("SELECT member,SUM(total_bytes) c FROM hook_runs WHERE ?1 IS NOT 0 AND last_ts>=?2 GROUP BY member HAVING c>0 ORDER BY c DESC, member LIMIT {}", o.top),
            o,
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )?
        .into_iter()
        .map(|(n, c)| (n.chars().filter(|c| !c.is_control()).collect(), c))
        .collect();
    }
    s.audit = words(
        conn,
        "admin_audit",
        "SELECT action,COUNT(*) c FROM admin_audit WHERE ?1 IS NOT 0 AND ts>=?2 GROUP BY action ORDER BY c DESC, 1",
        o,
    )?;
    if has(conn, "credentials")? {
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
    }
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
