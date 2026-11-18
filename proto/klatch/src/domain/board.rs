//! Offers and member status, derived from the message log like asks (DESIGN section 25.5). An offer is a
//! message starting `offering:`; it stays current until its sender withdraws it (`withdrawn:`), says
//! `status: done`, or it is older than the backstop lifetime. The gateway also drops an offer whose sender has
//! not been seen for a while (DESIGN section 25.8).

use crate::domain::hook::clip;
use crate::domain::{MemberInfo, MemberKind, Message, OpenAsk, Parked, flat};

/// A message whose body starts with this text is an offer.
pub const OFFER_PREFIX: &str = "offering:";
/// A message starting with this text withdraws its sender's current offer.
pub const WITHDRAW_PREFIX: &str = "withdrawn:";
/// A message starting with this text sets its sender's status.
pub const STATUS_PREFIX: &str = "status:";
/// A member not seen for this long is counted on the board, not listed.
pub const MEMBER_ACTIVE_SECS: i64 = 600;
/// Members not seen for this long are removed when a session starts (their messages stay).
pub const MEMBER_RETENTION_SECS: i64 = 86_400;
/// The most entries the board lists per section; the rest are counted.
pub const BOARD_LIST_LIMIT: usize = 20;
/// The longest an offer stays on the board whatever its sender does: a backstop. An offer normally follows its
/// sender's presence (section 25.8), so a member that stays present need not renew it.
pub const OFFER_TTL_SECS: i64 = 86_400;

/// The reader's clock: seconds since the epoch. A type of its own so the age helpers cannot be handed some other
/// number, and so an age is never negative when the reader's clock is behind the writer's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Now(i64);

impl Now {
    /// Whole seconds since `then`, never negative.
    fn since(self, then: i64) -> i64 {
        crate::domain::secs_between(self.0, then)
    }
}

/// What a member currently offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub sender: String,
    pub seq: i64,
    pub text: String,
    pub ts: i64,
}

/// The latest status a member announced (`status: <text>`), with the text after the prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberStatus {
    pub member: String,
    pub status: String,
}

/// A room member as the board shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardMember {
    pub name: String,
    pub last_seen: i64,
    /// The member's latest announced status text, if it ever announced one.
    pub status: Option<String>,
    /// The member whose session started this one, if known.
    pub parent: Option<String>,
    /// A declared long wait that still holds.
    pub parked: Option<Parked>,
    /// What sort of member this is (item 103), if known.
    pub kind: Option<MemberKind>,
}

/// The current state of a room derived from its log: open asks, current offers and members.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Board {
    pub room: String,
    pub asks: Vec<OpenAsk>,
    pub offers: Vec<Offer>,
    pub members: Vec<BoardMember>,
    /// Members not seen for over `MEMBER_ACTIVE_SECS`, counted instead of listed.
    pub stale_members: usize,
    /// Open durable tasks (working, checkpoint, handoff), with owners that are gone marked orphaned.
    pub tasks: Vec<crate::domain::Task>,
    /// Pull requests said to be open (item 84), with who holds each one's `pr:N` claim, if anyone.
    pub prs: Vec<crate::domain::OpenPr>,
}

/// `klatch who` as text: each member with last seen and status. Unless `all`, members not seen for
/// `MEMBER_ACTIVE_SECS` are counted in one line instead of listed, as on the board.
pub fn render_who(members: &[MemberInfo], now: i64, all: bool) -> String {
    let now = Now(now);
    let mut out = String::new();
    let mut gone = 0;
    for m in members {
        if !all && is_stale(m, now) {
            gone += 1;
            continue;
        }
        out.push_str(&who_line(m, now));
        out.push('\n');
    }
    match gone {
        0 => {}
        1 => out.push_str("1 other not seen for over 10 minutes (who --all lists them)\n"),
        n => out.push_str(&format!(
            "{n} others not seen for over 10 minutes (who --all lists them)\n"
        )),
    }
    out
}

/// Not seen for over `MEMBER_ACTIVE_SECS` and not holding a park.
fn is_stale(m: &MemberInfo, now: Now) -> bool {
    now.0 - m.last_seen > MEMBER_ACTIVE_SECS && !holds_park(m.parked.as_ref(), now)
}

/// One member's `who` line, without the trailing newline.
fn who_line(m: &MemberInfo, now: Now) -> String {
    let mut out = format!(
        "{}\tlast seen {}s ago (liveness, not progress)",
        flat(&m.name),
        now.since(m.last_seen)
    );
    if let Some(parent) = &m.parent {
        out.push_str(&format!("\tchild of {}", flat(parent)));
    }
    if let Some(kind) = m.kind {
        out.push_str(&format!("\tkind: {}", kind.word()));
    }
    if let Some(status) = &m.status {
        out.push_str(&format!("\tstatus: {}", flat(status)));
    }
    if let Some(p) = m.parked.as_ref().filter(|p| p.holds(now.0)) {
        out.push_str(&format!("\t{}", flat(&p.phrase(now.0))));
    }
    out
}

/// A parked member whose return time has not come stays listed however long since it was seen.
fn holds_park(parked: Option<&Parked>, now: Now) -> bool {
    parked.is_some_and(|p| p.holds(now.0))
}

/// A section heading, with a line naming the section when it is empty so a reader can see there is nothing.
fn section(out: &mut String, heading: &str, empty: Option<&str>) {
    out.push_str(heading);
    out.push('\n');
    if let Some(none) = empty {
        out.push_str(&format!("  {none}\n"));
    }
}

fn render_asks(board: &Board, now: Now, out: &mut String) {
    let none = board.asks.is_empty().then_some("no open asks");
    section(out, "asks:", none);
    for a in board.asks.iter().take(BOARD_LIST_LIMIT) {
        let summary = a.summary(now.0);
        let gap = if summary.is_empty() { "" } else { " " };
        let state = if a.claimed { "claimed" } else { "unclaimed" };
        out.push_str(&format!(
            "  #{} {} {state} {}s{gap}{summary}: {}\n",
            a.seq,
            flat(&a.sender),
            now.since(a.ts),
            clip(&a.text)
        ));
    }
    more(out, board.asks.len(), "asks");
}

fn render_offers(board: &Board, now: Now, out: &mut String) {
    let none = board.offers.is_empty().then_some("no current offers");
    section(out, "offers:", none);
    for o in board.offers.iter().take(BOARD_LIST_LIMIT) {
        out.push_str(&format!(
            "  {} offers ({}s ago): {}\n",
            flat(&o.sender),
            now.since(o.ts),
            clip(&o.text)
        ));
    }
    more(out, board.offers.len(), "offers");
}

/// A section that exists only when it has lines, bounded like the others.
fn render_list(out: &mut String, heading: &str, lines: &[String], what: &str) {
    if lines.is_empty() {
        return;
    }
    section(out, heading, None);
    for line in lines.iter().take(BOARD_LIST_LIMIT) {
        out.push_str(&format!("  {line}\n"));
    }
    more(out, lines.len(), what);
}

fn render_members(board: &Board, now: Now, out: &mut String) {
    section(out, "members:", None);
    for m in board.members.iter().take(BOARD_LIST_LIMIT) {
        let status = m
            .status
            .as_deref()
            .map(|s| format!(", status: {}", clip(s)))
            .unwrap_or_default();
        let parked = m
            .parked
            .as_ref()
            .filter(|p| p.holds(now.0))
            .map(|p| format!(", {}", clip(&p.phrase(now.0))))
            .unwrap_or_default();
        let parent = m
            .parent
            .as_deref()
            .map(|p| format!(", child of {}", flat(p)))
            .unwrap_or_default();
        let kind = m
            .kind
            .map(|k| format!(", kind: {}", k.word()))
            .unwrap_or_default();
        out.push_str(&format!(
            "  {} last seen {}s ago{parent}{kind}{status}{parked}\n",
            flat(&m.name),
            now.since(m.last_seen)
        ));
    }
    more(out, board.members.len(), "members");
    match board.stale_members {
        0 => {}
        1 => out.push_str("  1 other not seen for over 10 minutes\n"),
        n => out.push_str(&format!("  {n} others not seen for over 10 minutes\n")),
    }
}

/// The board as text for a person or an agent: one bounded line per entry, with ask and offer text treated
/// as data. Empty sections are named so a reader can see there is nothing.
pub fn render_board(board: &Board, now: i64) -> String {
    let now = Now(now);
    let mut out = format!("[board {}]\n", flat(&board.room));
    render_asks(board, now, &mut out);
    render_offers(board, now, &mut out);
    let tasks: Vec<String> = board.tasks.iter().map(|t| t.line(now.0, false)).collect();
    render_list(&mut out, "tasks:", &tasks, "tasks");
    let prs: Vec<String> = board.prs.iter().map(|p| p.line(now.0)).collect();
    render_list(&mut out, "prs:", &prs, "prs");
    render_members(board, now, &mut out);
    out
}

/// A line saying how many entries of a section were not listed.
fn more(out: &mut String, total: usize, what: &str) {
    if total > BOARD_LIST_LIMIT {
        out.push_str(&format!("  and {} more {what}\n", total - BOARD_LIST_LIMIT));
    }
}

pub fn is_offer(body: &str) -> bool {
    body.starts_with(OFFER_PREFIX)
}

/// The text after `status:` in a status message, trimmed.
fn status_text(body: &str) -> Option<&str> {
    body.strip_prefix(STATUS_PREFIX).map(str::trim)
}

/// The current offers in `messages` at time `now`, one per sender at most, oldest first.
pub fn current_offers(messages: &[Message], now: i64, ttl: i64) -> Vec<Offer> {
    let mut ordered: Vec<&Message> = messages.iter().collect();
    ordered.sort_by_key(|m| m.seq);
    let mut by_sender: Vec<(String, Option<Offer>)> = Vec::new();
    for m in ordered {
        let next = if is_offer(&m.body) {
            Some(Some(Offer {
                sender: m.sender.clone(),
                seq: m.seq,
                text: m.body.clone(),
                ts: m.ts,
            }))
        } else if m.body.starts_with(WITHDRAW_PREFIX)
            || status_text(&m.body).is_some_and(|t| t.starts_with("done"))
        {
            Some(None)
        } else {
            None
        };
        let Some(next) = next else { continue };
        match by_sender.iter_mut().find(|(s, _)| *s == m.sender) {
            Some(slot) => slot.1 = next,
            None => by_sender.push((m.sender.clone(), next)),
        }
    }
    let mut offers: Vec<Offer> = by_sender
        .into_iter()
        .filter_map(|(_, o)| o)
        .filter(|o| now - o.ts <= ttl)
        .collect();
    offers.sort_by_key(|o| o.seq);
    offers
}

/// Each member's latest announced status, in order of the member's first status message.
pub fn statuses(messages: &[Message]) -> Vec<MemberStatus> {
    let mut ordered: Vec<&Message> = messages.iter().collect();
    ordered.sort_by_key(|m| m.seq);
    let mut out: Vec<MemberStatus> = Vec::new();
    for m in ordered {
        let Some(text) = status_text(&m.body) else {
            continue;
        };
        match out.iter_mut().find(|s| s.member == m.sender) {
            Some(slot) => slot.status = text.to_string(),
            None => out.push(MemberStatus {
                member: m.sender.clone(),
                status: text.to_string(),
            }),
        }
    }
    out
}

/// What an asker should be told right after posting an ask: whether anyone could be reading it (#224). Option A of
/// the ticket made automatic, because the asker that did not check waited minutes for a helper that had left. A
/// member counts as reachable when it is not the asker, has not said it is done, and was seen within
/// `MEMBER_ACTIVE_SECS` or holds a declared wait. `None` when someone is reachable (the common case prints nothing).
pub fn ask_reach_note(members: &[MemberInfo], me: &str, now: i64) -> Option<String> {
    let reachable = members.iter().filter(|m| m.name != me).any(|m| {
        let done = m.status.as_deref().is_some_and(|s| s.starts_with("done"));
        !done
            && (now - m.last_seen <= MEMBER_ACTIVE_SECS || holds_park(m.parked.as_ref(), Now(now)))
    });
    (!reachable).then(|| {
        "klatch: no other member of this room has been seen in the last 10 minutes (or all said done). Your ask is \
         saved and a helper that joins later sees it at session start, but do not wait on it: run `klatch board` to \
         see who is around, or tell your human what is missing."
            .to_string()
    })
}

#[cfg(test)]
mod tests;
