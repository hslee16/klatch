//! The human's one-screen summary (#1414): who is doing what, what is stuck, and the commands to stop it.
//!
//! A pure fold over reads the caller was already allowed to make (`board`, `claims`, `stuck`), so nothing here can
//! reach what those reads refuse. Everything shown is data from other agents: names keep the plain alphabet, text is
//! stripped of control and bidirectional characters and clipped, every section lists a few rows and counts the rest,
//! and the whole text has a line and a byte cap. Message bodies and claim reasons are never printed. The intervention
//! lines are text for the human to copy; nothing here runs them, and a name or room that is not in the plain alphabet
//! gets no command at all, so no copied line can carry a shell word another agent chose.

use crate::domain::{Board, Claim, StuckEvent, secs_between, valid_name};

/// The longest a line may be, in characters.
pub const SUMMARY_LINE: usize = 100;
/// The byte cap of the listed rooms (the intervention footer is a fixed size on top).
pub const SUMMARY_MAX_BYTES: usize = 8192;
/// The line cap of the listed rooms.
pub const SUMMARY_MAX_LINES: usize = 60;
/// Rooms listed; the rest are counted.
pub const SUMMARY_ROOMS: usize = 4;
const MEMBER_ROWS: usize = 6;
const ASK_ROWS: usize = 4;
const CLAIM_ROWS: usize = 5;
const STUCK_ROWS: usize = 4;
const STATUS_CHARS: usize = 60;
const PATH_CHARS: usize = 48;

/// What the summary reads of one room: its board, its claims and its stuck cases.
pub struct SummaryRoom {
    pub room: String,
    pub board: Board,
    pub claims: Vec<Claim>,
    pub stuck: Vec<StuckEvent>,
}

/// A name in the plain alphabet (letters, digits, dot, dash, underscore), at most 40 characters; anything else is `?`.
fn name(s: &str) -> String {
    s.chars()
        .take(40)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '?'
            }
        })
        .collect()
}

/// Text with control characters as spaces, bidirectional and zero-width characters removed, cut to `max` characters.
fn plain(s: &str, max: usize) -> String {
    let mut out: String = s
        .chars()
        .filter(|c| {
            !matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200B}'..='\u{200F}'
                | '\u{061C}' | '\u{FEFF}')
        })
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .take(max + 1)
        .collect();
    if out.chars().count() > max {
        out = out.chars().take(max.saturating_sub(2)).collect();
        out.push_str("..");
    }
    out
}

/// One row, clipped to the line cap.
fn row(text: String) -> String {
    format!("  {}\n", plain(&text, SUMMARY_LINE - 2))
}

/// A section heading, its rows (at most `cap`) and a `+N more` line for the rest.
fn section(out: &mut String, heading: &str, rows: Vec<String>, cap: usize) {
    out.push_str(&format!("{}\n", plain(heading, SUMMARY_LINE)));
    if rows.is_empty() {
        out.push_str("  none\n");
    }
    let total = rows.len();
    for r in rows.into_iter().take(cap) {
        out.push_str(&row(format!(" {r}")));
    }
    if total > cap {
        out.push_str(&row(format!(" +{} more", total - cap)));
    }
}

fn render_room(r: &SummaryRoom, now: i64) -> String {
    let mut out = format!("[{}]\n", name(&r.room));
    let mut members: Vec<_> = r.board.members.iter().collect();
    members.sort_by_key(|m| std::cmp::Reverse(m.last_seen));
    let rows = members
        .iter()
        .map(|m| {
            let status = m.status.as_deref().map_or(String::new(), |s| {
                format!("  status: {}", plain(s, STATUS_CHARS))
            });
            let parked = m
                .parked
                .as_ref()
                .filter(|p| p.holds(now))
                .map_or("", |_| "  parked");
            format!(
                "{}  seen {}s ago{parked}{status}",
                name(&m.name),
                secs_between(now, m.last_seen)
            )
        })
        .collect();
    let gone = match r.board.stale_members {
        0 => String::new(),
        n => format!(", {n} not seen for over 10 minutes"),
    };
    section(
        &mut out,
        &format!("members ({} active{gone})", r.board.members.len()),
        rows,
        MEMBER_ROWS,
    );
    let mut asks: Vec<_> = r.board.asks.iter().collect();
    asks.sort_by_key(|a| (a.ts, a.seq));
    let rows = asks
        .iter()
        .map(|a| {
            let state = if a.claimed { "claimed" } else { "unclaimed" };
            format!(
                "#{} {} {state} {}s",
                a.seq,
                name(&a.sender),
                secs_between(now, a.ts)
            )
        })
        .collect();
    section(
        &mut out,
        &format!("open asks ({})", r.board.asks.len()),
        rows,
        ASK_ROWS,
    );
    let (live, lapsed): (Vec<&Claim>, Vec<&Claim>) = r.claims.iter().partition(|c| c.expires > now);
    let rows = live
        .iter()
        .map(|c| {
            format!(
                "{}  {}  {}s left",
                plain(&c.path, PATH_CHARS),
                name(&c.owner),
                c.expires - now
            )
        })
        .collect();
    let lapsed = match lapsed.len() {
        0 => String::new(),
        n => format!(", {n} lapsed not shown"),
    };
    section(
        &mut out,
        &format!("live claims ({}{lapsed})", live.len()),
        rows,
        CLAIM_ROWS,
    );
    let rows = r.stuck.iter().map(StuckEvent::line).collect();
    section(
        &mut out,
        &format!("stuck ({})", r.stuck.len()),
        rows,
        STUCK_ROWS,
    );
    out
}

/// The commands the human can run to step in, for the first things the summary shows. Text only.
fn interventions(rooms: &[SummaryRoom], now: i64) -> String {
    let mut lines: Vec<String> = Vec::new();
    for r in rooms.iter().filter(|r| valid_name(&r.room)) {
        if let Some(c) = r
            .claims
            .iter()
            .find(|c| c.expires > now && valid_name(&c.owner))
        {
            lines.push(format!(
                "klatch admin remove {} --room {}   (releases its claims there; undo with admin allow)",
                c.owner, r.room
            ));
        }
        if let Some(a) = r
            .board
            .asks
            .iter()
            .filter(|a| valid_name(&a.sender))
            .min_by_key(|a| (a.ts, a.seq))
        {
            lines.push(format!(
                "klatch answer {} {} \"closed by the human\"   (answers, so the ask closes)",
                r.room, a.seq
            ));
        }
        if lines.len() >= 4 {
            break;
        }
    }
    if let Some(r) = rooms.iter().find(|r| valid_name(&r.room)) {
        lines.push(format!(
            "klatch mute {}   (quiets your own hook in that room)",
            r.room
        ));
    }
    lines.push("klatch admin pause   (stops every room; klatch admin resume starts them)".into());
    let mut out = String::from(
        "to intervene (copy a line and run it yourself; admin acts ask for the passphrase; this view runs nothing):\n",
    );
    for l in lines {
        out.push_str(&format!("  {l}\n"));
    }
    out
}

/// The summary text. `not_shown` counts rooms the caller could not read or that are private working rooms; they are
/// named nowhere. `now` is the reader's clock.
pub fn render_summary(rooms: &[SummaryRoom], not_shown: usize, now: i64) -> String {
    let listed = &rooms[..rooms.len().min(SUMMARY_ROOMS)];
    let mut body = String::new();
    for r in listed {
        body.push_str(&render_room(r, now));
    }
    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    let mut out = String::new();
    let (mut kept, mut bytes) = (0, 0);
    for l in &lines {
        if kept >= SUMMARY_MAX_LINES || bytes + l.len() > SUMMARY_MAX_BYTES {
            break;
        }
        out.push_str(l);
        kept += 1;
        bytes += l.len();
    }
    if kept < lines.len() {
        out.push_str(&format!(
            "  +{} more lines not shown (klatch summary ROOM shows one room)\n",
            lines.len() - kept
        ));
    }
    if rooms.len() > listed.len() {
        out.push_str(&format!(
            "+{} more rooms (klatch summary ROOM shows one)\n",
            rooms.len() - listed.len()
        ));
    }
    if not_shown > 0 {
        out.push_str(&format!(
            "{not_shown} other rooms not shown (private, unreadable to you, dms, ask rooms or past the first few)\n"
        ));
    }
    out.push_str(&interventions(listed, now));
    out
}

#[cfg(test)]
#[path = "summary_tests.rs"]
mod tests;
