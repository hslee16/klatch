//! How a `UsageStats` is written: plain text for `klatch admin stats` and JSON for the benchmark ledger (#1377). Both
//! are built one section at a time, so each section of the report reads on its own. Counts and names only.

use super::{AskCounts, ClaimHistory, Counts, RoomClass, UsageStats, day_of};
#[cfg(feature = "client")]
use serde_json::{Value, json};
use std::fmt::Write;

fn table(out: &mut String, title: &str, head: [&str; 2], rows: &[(String, i64)]) {
    out.push_str(&format!("\n{title}\n"));
    if rows.is_empty() {
        out.push_str("  (none)\n");
        return;
    }
    let w = rows
        .iter()
        .map(|(k, _)| k.chars().count())
        .chain([head[0].chars().count()])
        .max()
        .unwrap_or(0);
    out.push_str(&format!("  {:<w$}  {}\n", head[0], head[1]));
    for (k, n) in rows {
        out.push_str(&format!("  {k:<w$}  {n}\n"));
    }
}

fn opt(n: Option<i64>) -> String {
    n.map_or("-".into(), |n| format!("{n}s"))
}

/// Appends formatted text to a `String`; writing to a `String` cannot fail.
fn put(out: &mut String, args: std::fmt::Arguments) {
    out.write_fmt(args).expect("writing to a String");
}

fn text_header(s: &UsageStats, o: &mut String) {
    o.push_str(
        "klatch stats (counts only; no message text, ask text, claim detail, label or payload)\n",
    );
    put(
        o,
        format_args!(
            "schema {}; room {}; since {}; at {}\n",
            s.schema,
            s.room.as_deref().unwrap_or("all"),
            s.since.map_or("the start".into(), day_of),
            day_of(s.now)
        ),
    );
}

fn text_rooms(s: &UsageStats, o: &mut String) {
    o.push_str("\nmessages per room\n");
    if s.rooms.is_empty() {
        o.push_str("  (none)\n");
    } else {
        let w = s
            .rooms
            .iter()
            .map(|r| r.room.chars().count())
            .max()
            .unwrap_or(4)
            .max(4);
        put(
            o,
            format_args!(
                "  {:<w$}  {:<7}  {:>8}  {:>6}  {:>7}  {:<10}  {}\n",
                "room", "class", "messages", "sealed", "senders", "first", "last"
            ),
        );
        for r in &s.rooms {
            put(
                o,
                format_args!(
                    "  {:<w$}  {:<7}  {:>8}  {:>6}  {:>7}  {:<10}  {}\n",
                    r.room,
                    r.class.word(),
                    r.messages,
                    r.sealed,
                    r.senders,
                    day_of(r.first_ts),
                    day_of(r.last_ts)
                ),
            );
        }
    }
    table(o, "messages per day (UTC)", ["day", "messages"], &s.days);
}

fn text_senders(s: &UsageStats, o: &mut String) {
    put(
        o,
        format_args!(
            "\nsenders: {} distinct, {} with a verified message\n",
            s.senders_distinct, s.verified_senders
        ),
    );
    table(o, "top senders", ["sender", "messages"], &s.senders_top);
    let by = ["attribution", "messages"];
    table(o, "attribution", by, &s.attribution);
    table(o, "message kinds", ["kind", "messages"], &s.kinds);
}

fn text_asks(a: &AskCounts, o: &mut String) {
    put(
        o,
        format_args!(
            "\nasks: {} created, {} answered, {} closed, {} with a declared scope, {} with refs\n",
            a.created, a.answered, a.closed, a.with_scope, a.with_refs
        ),
    );
    put(
        o,
        format_args!(
            "time to first answer: median {}, p90 {}\n",
            opt(a.first_answer_median),
            opt(a.first_answer_p90)
        ),
    );
    if a.sealed_unread > 0 {
        put(
            o,
            format_args!(
                "{} messages under a sealed label were not read for asks\n",
                a.sealed_unread
            ),
        );
    }
    table(o, "ask refs by kind", ["kind", "refs"], &a.ref_kinds);
}

fn text_claims(s: &UsageStats, o: &mut String) {
    let c = &s.claims;
    table(o, "live claims by class", ["class", "claims"], &c.live);
    table(o, "claim notes by class", ["class", "notes"], &c.notes);
    table(
        o,
        "members waiting on a held claim now, by class",
        ["class", "waiting"],
        &c.waiting,
    );
    put(o, format_args!("claim yields on record: {}\n", c.yields));
    text_claim_history(&c.history, o);
}

fn text_claim_history(h: &ClaimHistory, o: &mut String) {
    table(
        o,
        "claim grants by class (new holds; renewals are not counted)",
        ["class", "grants"],
        &h.granted,
    );
    table(
        o,
        "claim refusals by rule",
        ["rule", "refusals"],
        &h.refused,
    );
    table(
        o,
        "claim releases by class",
        ["class", "releases"],
        &h.released,
    );
    table(
        o,
        "claim releases by who released",
        ["by", "releases"],
        &h.released_by,
    );
    table(
        o,
        "claim expiries by class (the holder's lease lapsed)",
        ["class", "expiries"],
        &h.expired,
    );
    put(
        o,
        format_args!(
            "claims taken over after expiry: {}\nclaim conflicts handled (refused, then granted): {}, median wait {}\nclaim hold time: median {} over {} holds\n",
            h.takeovers,
            h.handled,
            opt(h.handled_wait_median),
            opt(h.hold_median),
            h.holds
        ),
    );
}

fn text_machine(s: &UsageStats, o: &mut String) {
    let mc = &s.machine_checkpoints;
    put(
        o,
        format_args!(
            "machine checkpoints: {} written ({} at session end), {} replaced an earlier one, {} bytes\n",
            mc.written, mc.last, mc.replaced, mc.bytes
        ),
    );
    let ms = &s.machine_status;
    put(
        o,
        format_args!(
            "machine status: {} updates (working {}, idle {}, parked {})\n",
            ms.updates, ms.working, ms.idle, ms.parked
        ),
    );
    let pc = &s.passive_claims;
    put(
        o,
        format_args!(
            "passive claims: {} granted, {} renewed, {} refused, {} released, {} lapsed, {} failed\n",
            pc.granted, pc.renewed, pc.refused, pc.released, pc.expired, pc.errors
        ),
    );
    let ar = &s.ask_routing;
    put(
        o,
        format_args!(
            "ask routing: {} routed to capable members, {} routed to nobody, {} taken, {} lapsed, {} escalated ({} with no coordinator to reach)\n",
            ar.routed, ar.unrouted, ar.taken, ar.lapsed, ar.escalated, ar.escalated_unreached
        ),
    );
}

/// How many rooms of a class there are and how many messages they hold.
fn class_totals(s: &UsageStats, class: RoomClass) -> (i64, i64) {
    s.rooms
        .iter()
        .filter(|r| r.class == class)
        .fold((0, 0), |(a, b), r| (a + 1, b + r.messages))
}

fn text_room_classes(s: &UsageStats, o: &mut String) {
    let (dm_rooms, dm_msgs) = class_totals(s, RoomClass::Dm);
    let (pv_rooms, pv_msgs) = class_totals(s, RoomClass::Private);
    put(
        o,
        format_args!(
            "\ndirect message rooms: {dm_rooms} ({dm_msgs} messages); private rooms: {pv_rooms} ({pv_msgs} messages)\n"
        ),
    );
}

fn text_members(s: &UsageStats, o: &mut String) {
    let m = &s.members;
    put(
        o,
        format_args!(
            "members: {} joined, {} never posted, {} marked done, {} muted, {} with a parent\n",
            m.joined, m.never_posted, m.marked_done, m.muted, m.with_parent
        ),
    );
    put(
        o,
        format_args!(
            "members who posted and never read: {} of {} ({} never shown anything by a hook, wait or wake)\n",
            m.never_read, m.posted, m.never_shown
        ),
    );
    if !m.never_read_days.is_empty() {
        let days: Vec<String> = m
            .never_read_days
            .iter()
            .map(|(d, n)| format!("{d} {n}"))
            .collect();
        put(
            o,
            format_args!("  by day of first post: {}\n", days.join(", ")),
        );
    }
}

/// The count for `event` in a per-event table, 0 when the event is not there.
fn count_of(rows: &Counts, event: &str) -> i64 {
    rows.iter().find(|(e, _)| e == event).map_or(0, |(_, n)| *n)
}

fn text_hooks(s: &UsageStats, o: &mut String) {
    table(
        o,
        "hook runs by event (cumulative)",
        ["event", "runs"],
        &s.hooks,
    );
    o.push_str(
        "\nbytes injected into sessions by hook event (sizes only; the average is per run)\n",
    );
    if s.hook_bytes.is_empty() {
        o.push_str("  (none recorded)\n");
    }
    for (event, total) in &s.hook_bytes {
        let runs = count_of(&s.hooks, event);
        let max = count_of(&s.hook_bytes_max, event);
        let avg = if runs > 0 { total / runs } else { 0 };
        put(
            o,
            format_args!(
                "  {event}: {total} bytes in {runs} runs (average {avg}, largest {max})\n"
            ),
        );
    }
    table(
        o,
        "largest sessions by bytes injected",
        ["session", "bytes"],
        &s.sessions_top_bytes,
    );
}

impl UsageStats {
    /// The report as stable plain text: fixed section order, sorted rows, no timestamp that varies between runs of the
    /// same store except the stated moment.
    pub fn to_text(&self) -> String {
        let mut o = String::new();
        text_header(self, &mut o);
        text_rooms(self, &mut o);
        text_senders(self, &mut o);
        text_asks(&self.asks, &mut o);
        text_claims(self, &mut o);
        text_machine(self, &mut o);
        table(
            &mut o,
            "refusals by rule",
            ["rule", "refusals"],
            &self.refusals,
        );
        text_room_classes(self, &mut o);
        text_members(self, &mut o);
        text_hooks(self, &mut o);
        table(&mut o, "admin audit by act", ["act", "count"], &self.audit);
        put(
            &mut o,
            format_args!(
                "\nworker credentials: {} issued, {} revoked\n",
                self.credentials_issued, self.credentials_revoked
            ),
        );
        o
    }
}

#[cfg(feature = "client")]
fn pairs(c: &Counts) -> Value {
    Value::Array(
        c.iter()
            .map(|(k, n)| json!({"name": k, "count": n}))
            .collect(),
    )
}

#[cfg(feature = "client")]
fn json_asks(a: &AskCounts) -> Value {
    json!({
        "created": a.created, "answered": a.answered, "closed": a.closed,
        "with_scope": a.with_scope, "with_refs": a.with_refs,
        "ref_kinds": pairs(&a.ref_kinds),
        "first_answer_median_secs": a.first_answer_median,
        "first_answer_p90_secs": a.first_answer_p90,
        "sealed_unread": a.sealed_unread,
    })
}

#[cfg(feature = "client")]
fn json_claims(s: &UsageStats) -> Value {
    let (c, h) = (&s.claims, &s.claims.history);
    json!({
        "live": pairs(&c.live), "notes": pairs(&c.notes),
        "waiting": pairs(&c.waiting), "yields": c.yields,
        "history": {
            "granted": pairs(&h.granted),
            "refused": pairs(&h.refused),
            "released": pairs(&h.released),
            "released_by": pairs(&h.released_by),
            "expired": pairs(&h.expired),
            "takeovers": h.takeovers,
            "conflicts_handled": h.handled,
            "conflict_wait_median_secs": h.handled_wait_median,
            "hold_median_secs": h.hold_median,
            "holds": h.holds,
        },
    })
}

#[cfg(feature = "client")]
fn json_machine(s: &UsageStats) -> [(&'static str, Value); 4] {
    let (mc, ms, pc, ar) = (
        &s.machine_checkpoints,
        &s.machine_status,
        &s.passive_claims,
        &s.ask_routing,
    );
    [
        (
            "machine_checkpoints",
            json!({
                "written": mc.written, "last": mc.last,
                "replaced": mc.replaced, "bytes": mc.bytes,
            }),
        ),
        (
            "machine_status",
            json!({
                "updates": ms.updates, "working": ms.working,
                "idle": ms.idle, "parked": ms.parked,
            }),
        ),
        (
            "passive_claims",
            json!({
                "granted": pc.granted, "renewed": pc.renewed,
                "refused": pc.refused, "released": pc.released,
                "lapsed": pc.expired, "failed": pc.errors,
            }),
        ),
        (
            "ask_routing",
            json!({
                "routed": ar.routed, "unrouted": ar.unrouted,
                "taken": ar.taken, "lapsed": ar.lapsed,
                "escalated": ar.escalated,
                "escalated_unreached": ar.escalated_unreached,
            }),
        ),
    ]
}

#[cfg(feature = "client")]
fn json_members(s: &UsageStats) -> Value {
    let m = &s.members;
    json!({
        "joined": m.joined, "never_posted": m.never_posted,
        "marked_done": m.marked_done, "muted": m.muted,
        "with_parent": m.with_parent,
        "posted": m.posted, "posted_never_read": m.never_read,
        "posted_never_shown": m.never_shown,
        "posted_never_read_by_day": pairs(&m.never_read_days),
    })
}

#[cfg(feature = "client")]
impl UsageStats {
    /// The report as one JSON object, counts and timestamps only, for the benchmark ledger.
    pub fn to_json(&self) -> Value {
        let mut top = json!({
            "schema": self.schema,
            "room": self.room,
            "since": self.since,
            "at": self.now,
            "rooms": self.rooms.iter().map(|r| json!({
                "room": r.room, "class": r.class.word(), "messages": r.messages, "sealed": r.sealed,
                "senders": r.senders, "first": r.first_ts, "last": r.last_ts,
            })).collect::<Vec<_>>(),
            "days": self.days.iter().map(|(d, n)| json!({"day": d, "messages": n})).collect::<Vec<_>>(),
            "room_days": self.room_days.iter().map(|(r, d, n)| json!({"room": r, "day": d, "messages": n})).collect::<Vec<_>>(),
            "senders": {"distinct": self.senders_distinct, "verified": self.verified_senders, "top": pairs(&self.senders_top)},
            "attribution": pairs(&self.attribution),
            "kinds": pairs(&self.kinds),
            "asks": json_asks(&self.asks),
            "claims": json_claims(self),
        });
        let map = top.as_object_mut().expect("a JSON object");
        map.extend(json_machine(self).map(|(k, v)| (k.to_string(), v)));
        map.insert("refusals".into(), pairs(&self.refusals));
        map.insert("members".into(), json_members(self));
        map.insert("hooks".into(), pairs(&self.hooks));
        map.insert("hook_bytes".into(), pairs(&self.hook_bytes));
        map.insert("hook_bytes_max".into(), pairs(&self.hook_bytes_max));
        map.insert("sessions_top_bytes".into(), pairs(&self.sessions_top_bytes));
        map.insert("audit".into(), pairs(&self.audit));
        map.insert(
            "credentials".into(),
            json!({"issued": self.credentials_issued, "revoked": self.credentials_revoked}),
        );
        top
    }
}
