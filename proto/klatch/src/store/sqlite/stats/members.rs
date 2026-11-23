//! The member section of the usage report (#1377, #1411): who joined, never posted, was marked done, muted, and who
//! posted and never read. Counts only; no name leaves this module.

use super::{has, one, rows};
use crate::domain::{MemberCounts, Result, StatsOptions, day_of};
use rusqlite::Connection;
use std::collections::BTreeMap;

/// The members in scope: in the room asked for (or any) and joined in the window.
const POP: &str = "(?1 IS NULL OR room=?1) AND joined>=?2";

/// One posting name: the furthest any of its rows read, the furthest it was shown, and the time of its first post.
struct Poster {
    read: i64,
    shown: i64,
    first: i64,
}

fn posters(conn: &Connection, o: &StatsOptions) -> Result<Vec<Poster>> {
    rows(
        conn,
        &format!(
            "SELECT MAX(COALESCE(last_seq,0)), MAX(MAX(hook_seq,wait_seq,wake_seq)), \
             (SELECT MIN(ts) FROM messages m WHERE m.sender=members.name AND m.kind IS NOT 'archived' AND (?1 IS NULL OR m.room=?1)) AS first \
             FROM members WHERE {POP} GROUP BY name HAVING first IS NOT NULL"
        ),
        o,
        |r| {
            Ok(Poster {
                read: r.get(0)?,
                shown: r.get(1)?,
                first: r.get(2)?,
            })
        },
    )
}

/// Distinct member names in scope that also satisfy `cond`, an extra condition on the `members` row.
fn names_where(conn: &Connection, o: &StatsOptions, cond: &str) -> Result<i64> {
    one(
        conn,
        &format!("SELECT COUNT(DISTINCT name) FROM members WHERE {POP}{cond}"),
        o,
    )
}

fn with_parent(conn: &Connection, o: &StatsOptions) -> Result<i64> {
    if !has(conn, "member_parent")? {
        return Ok(0);
    }
    names_where(conn, o, " AND name IN (SELECT member FROM member_parent)")
}

/// The posters whose read cursor never moved, by the day of their first post.
fn never_read_by_day(posters: &[Poster]) -> BTreeMap<String, i64> {
    let mut by_day: BTreeMap<String, i64> = BTreeMap::new();
    for p in posters.iter().filter(|p| p.read == 0) {
        *by_day.entry(day_of(p.first)).or_default() += 1;
    }
    by_day
}

fn count_of<T>(items: impl Iterator<Item = T>) -> i64 {
    i64::try_from(items.count()).unwrap_or(i64::MAX)
}

pub(super) fn members(conn: &Connection, o: &StatsOptions) -> Result<MemberCounts> {
    if !has(conn, "members")? {
        return Ok(MemberCounts::default());
    }
    let with_parent = with_parent(conn, o)?;
    let posters = posters(conn, o)?;
    let by_day = never_read_by_day(&posters);
    Ok(MemberCounts {
        posted: count_of(posters.iter()),
        never_read: by_day.values().sum(),
        never_shown: count_of(posters.iter().filter(|p| p.read == 0 && p.shown == 0)),
        never_read_days: by_day.into_iter().collect(),
        joined: names_where(conn, o, "")?,
        never_posted: names_where(
            conn,
            o,
            " AND NOT EXISTS (SELECT 1 FROM messages m WHERE m.sender=members.name AND m.kind IS NOT 'archived' AND (?1 IS NULL OR m.room=?1))",
        )?,
        marked_done: names_where(conn, o, " AND status LIKE 'done%'")?,
        muted: names_where(conn, o, " AND muted<>0")?,
        with_parent,
    })
}
