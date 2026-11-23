//! The room section of the usage report (#1377): messages per room, how many are sealed, and the kind of room.

use super::{LIVE, has, name_counts, rows};
use crate::domain::{DM_PREFIX, Result, RoomClass, RoomCount, StatsOptions};
use rusqlite::Connection;
use std::collections::{BTreeMap, HashMap, HashSet};

/// The label each room has been given by policy, by room.
fn room_labels(conn: &Connection, o: &StatsOptions) -> Result<HashMap<String, String>> {
    if !has(conn, "policy")? {
        return Ok(HashMap::new());
    }
    let sql = "SELECT substr(key,7), value FROM policy WHERE key LIKE 'label:%'";
    Ok(rows(conn, sql, o, |r| Ok((r.get(0)?, r.get(1)?)))?
        .into_iter()
        .collect())
}

/// The lowest label of the chain: a room labelled above it is private.
fn lowest_label(conn: &Connection, o: &StatsOptions) -> Result<String> {
    if !has(conn, "policy")? {
        return Ok("public".into());
    }
    let chain = rows(conn, "SELECT value FROM policy WHERE key='chain'", o, |r| {
        r.get::<_, String>(0)
    })?;
    let lowest = chain
        .first()
        .and_then(|c| c.split(',').next().map(|s| s.trim().to_string()));
    Ok(lowest.unwrap_or_else(|| "public".into()))
}

/// What a room's name and label make of it.
fn class_of(room: &str, labels: &HashMap<String, String>, lowest: &str) -> RoomClass {
    if room.starts_with(DM_PREFIX) {
        RoomClass::Dm
    } else if labels.get(room).is_some_and(|l| l.as_str() != lowest) {
        RoomClass::Private
    } else {
        RoomClass::Ordinary
    }
}

/// One group of live messages: a room, a label, how many and the first and last moment.
struct Group {
    room: String,
    label: String,
    n: i64,
    first: i64,
    last: i64,
}

fn groups(conn: &Connection, o: &StatsOptions) -> Result<Vec<Group>> {
    rows(
        conn,
        &format!(
            "SELECT room,label,COUNT(*),MIN(ts),MAX(ts) FROM messages WHERE {LIVE} GROUP BY room,label"
        ),
        o,
        |r| {
            Ok(Group {
                room: r.get(0)?,
                label: r.get(1)?,
                n: r.get(2)?,
                first: r.get(3)?,
                last: r.get(4)?,
            })
        },
    )
}

/// Folds the groups into one count per room, the messages under a sealed label counted apart.
fn fold_groups(
    groups: Vec<Group>,
    sealed: &HashSet<String>,
    classify: impl Fn(&str) -> RoomClass,
) -> BTreeMap<String, RoomCount> {
    let mut by_room: BTreeMap<String, RoomCount> = BTreeMap::new();
    for g in groups {
        let e = by_room.entry(g.room.clone()).or_insert_with(|| RoomCount {
            class: classify(&g.room),
            room: g.room,
            messages: 0,
            sealed: 0,
            senders: 0,
            first_ts: g.first,
            last_ts: g.last,
        });
        e.messages += g.n;
        if sealed.contains(&g.label) {
            e.sealed += g.n;
        }
        e.first_ts = e.first_ts.min(g.first);
        e.last_ts = e.last_ts.max(g.last);
    }
    by_room
}

pub(super) fn rooms(
    conn: &Connection,
    o: &StatsOptions,
    sealed: &HashSet<String>,
) -> Result<Vec<RoomCount>> {
    let labels = room_labels(conn, o)?;
    let lowest = lowest_label(conn, o)?;
    let groups = groups(conn, o)?;
    let mut by_room = fold_groups(groups, sealed, |room| class_of(room, &labels, &lowest));
    let senders = name_counts(
        conn,
        &format!("SELECT room,COUNT(DISTINCT sender) FROM messages WHERE {LIVE} GROUP BY room"),
        o,
    )?;
    for (room, n) in senders {
        if let Some(e) = by_room.get_mut(&room) {
            e.senders = n;
        }
    }
    let mut out: Vec<RoomCount> = by_room.into_values().collect();
    out.sort_by(|a, b| b.messages.cmp(&a.messages).then(a.room.cmp(&b.room)));
    Ok(out)
}
