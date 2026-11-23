//! The event-record sections of the usage report: claim history (#1386), machine checkpoints (#1408), machine status
//! (#1403), passive claims (#1401) and ask routing (#1407). Counts and seconds of closed vocabularies; no name and no
//! text is selected.

use super::{has, one, rows, words};
use crate::domain::{
    ClaimHistory, Counts, MachineCheckpointCounts, MachineStatusCounts, PassiveClaimCounts, Result,
    RoutingCounts, StatsOptions, median_and_p90,
};
use rusqlite::Connection;

/// The room and window condition every event table shares.
const WINDOW: &str = "(?1 IS NULL OR room=?1) AND ts>=?2";

/// `(word, count)` of `col` over the `claim_events` of one kind of event.
fn claim_words(conn: &Connection, o: &StatsOptions, col: &str, event: &str) -> Result<Counts> {
    words(
        conn,
        "claim_events",
        &format!(
            "SELECT {col},COUNT(*) c FROM claim_events WHERE event='{event}' AND (?1 IS NULL OR room=?1) AND ts>=?2 GROUP BY {col} ORDER BY c DESC, 1"
        ),
        o,
    )
}

/// The seconds in `col` of the claim events chosen by `cond`.
fn claim_seconds(conn: &Connection, o: &StatsOptions, col: &str, cond: &str) -> Result<Vec<i64>> {
    rows(
        conn,
        &format!("SELECT {col} FROM claim_events WHERE {cond} AND {WINDOW}"),
        o,
        |r| r.get::<_, i64>(0),
    )
}

/// The claim event record (#1386): counts by class and word from a closed vocabulary, and medians of seconds. Nothing
/// in the table is a name, so nothing here can print one.
pub(super) fn claim_history(conn: &Connection, o: &StatsOptions) -> Result<ClaimHistory> {
    let mut h = ClaimHistory {
        granted: claim_words(conn, o, "class", "granted")?,
        refused: claim_words(conn, o, "reason", "refused")?,
        released: claim_words(conn, o, "class", "released")?,
        released_by: claim_words(conn, o, "reason", "released")?,
        expired: claim_words(conn, o, "class", "expired")?,
        ..ClaimHistory::default()
    };
    if !has(conn, "claim_events")? {
        return Ok(h);
    }
    h.takeovers = one(
        conn,
        &format!("SELECT COUNT(*) FROM claim_events WHERE event='takeover' AND {WINDOW}"),
        o,
    )?;
    let waits = claim_seconds(conn, o, "waited", "event='granted' AND waited IS NOT NULL")?;
    h.handled = waits.len() as i64;
    h.handled_wait_median = median_and_p90(waits).0;
    let holds = claim_seconds(
        conn,
        o,
        "held",
        "event IN ('released','expired') AND held>0",
    )?;
    h.holds = holds.len() as i64;
    h.hold_median = median_and_p90(holds).0;
    Ok(h)
}

/// A counter over `table` in the window, with an extra condition appended to each query.
fn counter<'a>(
    conn: &'a Connection,
    o: &'a StatsOptions,
    table: &'a str,
) -> impl Fn(&str) -> Result<i64> + 'a {
    move |extra| {
        one(
            conn,
            &format!("SELECT COUNT(*) FROM {table} WHERE {WINDOW}{extra}"),
            o,
        )
    }
}

/// The machine checkpoint events (#1408) in the window: counts and bytes. Nothing in the table is text.
pub(super) fn machine_checkpoint_counts(
    conn: &Connection,
    o: &StatsOptions,
) -> Result<MachineCheckpointCounts> {
    let table = "machine_checkpoint_events";
    if !has(conn, table)? {
        return Ok(MachineCheckpointCounts::default());
    }
    let n = counter(conn, o, table);
    Ok(MachineCheckpointCounts {
        written: n("")?,
        last: n(" AND last<>0")?,
        replaced: n(" AND replaced<>0")?,
        bytes: one(
            conn,
            &format!("SELECT COALESCE(SUM(bytes),0) FROM {table} WHERE {WINDOW}"),
            o,
        )?,
    })
}

/// The machine status updates (#1403) in the window, by class. Nothing in the table is text but a class word.
pub(super) fn machine_status_counts(
    conn: &Connection,
    o: &StatsOptions,
) -> Result<MachineStatusCounts> {
    let table = "machine_status_events";
    if !has(conn, table)? {
        return Ok(MachineStatusCounts::default());
    }
    let n = counter(conn, o, table);
    Ok(MachineStatusCounts {
        updates: n("")?,
        working: n(" AND class='working'")?,
        idle: n(" AND class='idle'")?,
        parked: n(" AND class='parked'")?,
    })
}

/// The passive claim events (#1401) in the window: counts by event word. Nothing in the table is a name.
pub(super) fn passive_claim_counts(
    conn: &Connection,
    o: &StatsOptions,
) -> Result<PassiveClaimCounts> {
    if !has(conn, "claim_events")? {
        return Ok(PassiveClaimCounts::default());
    }
    let n = |event: &str| {
        one(
            conn,
            &format!(
                "SELECT COUNT(*) FROM claim_events WHERE event='{event}' AND (?1 IS NULL OR room=?1) AND ts>=?2"
            ),
            o,
        )
    };
    Ok(PassiveClaimCounts {
        granted: n("passive_granted")?,
        renewed: n("passive_renewed")?,
        refused: n("passive_refused")?,
        released: n("passive_released")?,
        expired: n("passive_expired")?,
        errors: n("passive_error")?,
    })
}

/// The ask routing events (#1407) in the window: asks routed, taken, lapsed and escalated. One row per ask and outcome,
/// so these are counts of asks. Nothing in the table is text.
pub(super) fn ask_routing_counts(conn: &Connection, o: &StatsOptions) -> Result<RoutingCounts> {
    if !has(conn, "ask_route_events")? {
        return Ok(RoutingCounts::default());
    }
    let n = |outcome: &str, extra: &str| {
        one(
            conn,
            &format!(
                "SELECT COUNT(*) FROM ask_route_events WHERE outcome='{outcome}' AND (?1 IS NULL OR room=?1) AND ts>=?2{extra}"
            ),
            o,
        )
    };
    Ok(RoutingCounts {
        routed: n("routed", " AND n>0")?,
        unrouted: n("routed", " AND n=0")?,
        taken: n("taken", "")?,
        lapsed: n("lapsed", "")?,
        escalated: n("escalated", "")?,
        escalated_unreached: n("escalated", " AND n=0")?,
    })
}
