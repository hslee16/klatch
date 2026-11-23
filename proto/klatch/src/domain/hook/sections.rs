//! The sections a room's digest prints beneath its messages: claims, notices, the member's own asks and the backlog.

use super::*;

fn render_claims(d: &RoomDigest, now: i64, out: &mut String) {
    for c in &d.claims {
        out.push_str(&format!(
            "  claim: {} holds {} for {}s more; avoid editing it\n",
            flat(&c.owner),
            flat(&c.path),
            crate::domain::secs_between(c.expires, now)
        ));
    }
    for r in &d.released {
        out.push_str(&format!(
            "  claim released: {} no longer holds {}",
            flat(&r.owner),
            flat(&r.path)
        ));
        if let Some(note) = r.note.as_deref().filter(|n| !n.is_empty()) {
            out.push_str(&format!(": {} (re-read it before you edit)", clip(note)));
        }
        out.push('\n');
    }
}

/// The member's own claims held past an age milestone: one line each, a path and a length of time, nothing else.
fn render_aged_claims(d: &RoomDigest, out: &mut String) {
    for a in &d.aged {
        out.push_str(&format!(
            "  your claim on {} has been held for {}; release it if you are done, or ignore this to keep it\n",
            flat(&a.path),
            crate::domain::parked::span(a.held_secs)
        ));
    }
}

/// A cycle of waits that a claim being taken completed: the same instruction a refusal that closes a cycle gives.
fn render_cycle_notices(d: &RoomDigest, out: &mut String) {
    for c in &d.cycles {
        out.push_str(&format!(
            "  deadlock: your wait for {} is now part of a cycle: {} took a claim and waits, directly or through others, for something you hold, so you yield: release the claims you hold in this room, wait, then try again\n",
            flat(&c.path),
            flat(&c.holder)
        ));
    }
}

/// The derived stop-waiting notes (#789): one line each, as the domain made them.
fn render_asks_notes(d: &RoomDigest, out: &mut String) {
    for n in &d.asks_notes {
        out.push_str(&format!("  {}\n", flat(n)));
    }
}

/// What became of the member's own crossings (#679): one line each, a state and the fixed reason, never the text.
fn render_crossing_notices(d: &RoomDigest, out: &mut String) {
    for c in &d.crossings {
        let head = format!(
            "  your crossing of message {} to {}:",
            c.seq,
            flat(&c.to_room)
        );
        match c.state.as_str() {
            "waiting" | "approved" => out.push_str(&format!(
                "{head} waiting for a decision; you will be told when it is made\n"
            )),
            "released" => out.push_str(&format!("{head} released\n")),
            _ => out.push_str(&format!("{head} refused, {}\n", flat(&c.why))),
        }
    }
}

/// The dm help lines (#1190, #1192): fixed templates over validated names, a count and the room, never message text.
/// A dm room names its peer and the reply command; a shared room whose two members exchanged a long thread suggests
/// moving it to a dm. A line is printed only when the room has new messages for the member.
fn render_dm_help(d: &RoomDigest, out: &mut String) {
    if d.messages.is_empty() {
        return;
    }
    if let Some(peer) = &d.dm_peer {
        out.push_str(&format!("  {}\n", crate::domain::dm_reply_hint(peer)));
    }
    if let Some((peer, n)) = &d.move_to_dm {
        out.push_str(&format!(
            "  {}\n",
            crate::domain::move_to_dm_nudge(peer, *n, &d.room)
        ));
    }
}

/// What a narrowed view held back (#673): a count and the newest seq, the same words the wake pointer uses.
fn render_outside_view(d: &RoomDigest, out: &mut String) {
    if let Some((n, newest)) = d.outside_view {
        out.push_str(&format!("  {}\n", outside_view_line(&d.room, n, newest)));
    }
}

/// "N messages outside your view in room R, newest #SEQ", with a way to read them. Counts only.
pub fn outside_view_line(room: &str, n: usize, newest: i64) -> String {
    let noun = if n == 1 { "message" } else { "messages" };
    format!(
        "{n} {noun} outside your view in room {}, newest #{newest}; `klatch read {} --peek` shows them",
        flat(room),
        flat(room)
    )
}

/// The member's own asks that need its attention: gone stale, escalated, or whose claim lapsed.
fn render_own_asks(d: &RoomDigest, now: i64, out: &mut String) {
    for a in &d.stale {
        out.push_str(&format!(
            "  your ask #{} has had no claim for {}s: {}\n",
            a.seq,
            crate::domain::secs_between(now, a.ts),
            clip(&a.text)
        ));
    }
    for a in &d.escalated {
        out.push_str(&format!(
            "  your ask #{}: nobody here may be able to do this ({} helpers declined as not able): {}\n",
            a.seq,
            a.distinct(crate::domain::OutcomeKind::Cannot),
            clip(&a.text)
        ));
    }
    for (a, owner) in &d.lapsed {
        out.push_str(&format!(
            "  your ask #{}: {}'s claim lapsed without an answer; the ask is open again: {}\n",
            a.seq,
            flat(owner),
            clip(&a.text)
        ));
    }
}

/// An open ask from another member, with what the next helper starts from: partial work already done and any
/// question the asker owes.
fn render_backlog_ask(a: &OpenAsk, out: &mut String) {
    out.push_str(&format!(
        "  open ask #{} from {} (no claim): {}\n",
        a.seq,
        flat(&a.sender),
        clip(&a.text)
    ));
    let partials = a
        .outcomes
        .iter()
        .filter(|o| o.kind == crate::domain::OutcomeKind::Partial);
    for o in partials {
        out.push_str(&format!(
            "    partial by {}: {}\n",
            flat(&o.by),
            clip(&o.reason)
        ));
    }
    if let Some(w) = &a.waiting_on_asker {
        out.push_str(&format!(
            "    waiting on {}: {}\n",
            flat(&a.sender),
            clip(&w.question)
        ));
    }
}

/// The room's lines other than messages (claims, the member's own asks, the backlog), one entry per item. An item
/// that takes several lines (a backlog ask with its partial work) stays together.
pub(super) fn other_units(d: &RoomDigest, now: i64) -> Vec<String> {
    let mut text = String::new();
    render_claims(d, now, &mut text);
    render_aged_claims(d, &mut text);
    render_cycle_notices(d, &mut text);
    render_asks_notes(d, &mut text);
    render_crossing_notices(d, &mut text);
    render_outside_view(d, &mut text);
    render_dm_help(d, &mut text);
    render_own_asks(d, now, &mut text);
    for a in &d.backlog {
        render_backlog_ask(a, &mut text);
    }
    for p in &d.unowned_prs {
        text.push_str(&format!("  {}\n", p.obligation(now)));
    }
    let mut units: Vec<String> = Vec::new();
    for line in text.split_inclusive('\n') {
        // a continuation line is indented deeper than an item line
        match units.last_mut() {
            Some(last) if line.starts_with("    ") => last.push_str(line),
            _ => units.push(line.to_string()),
        }
    }
    units
}
