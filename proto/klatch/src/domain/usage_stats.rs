//! The counts-only usage report behind `klatch admin stats` (#1377): the shape of the report, how claims are
//! classified, and how it is written as plain text and as JSON. This file holds no store access and no message text:
//! every field is a count, a timestamp, a room or member name, or a word from a closed system vocabulary (a rule, an
//! event, an admin act, a message kind) that `vocab` has checked. Nothing a member wrote (a body, an ask, a claim
//! reason or note, a label, a payload) can reach it.

#[cfg(feature = "client")]
use serde_json::{Value, json};

/// What the report is about: one room or all of them, from a moment or from the start.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatsOptions {
    pub room: Option<String>,
    /// Unix seconds; only rows at or after it are counted (hook runs are cumulative and are kept when their last run is).
    pub since: Option<i64>,
    /// How many senders to list, most messages first.
    pub top: usize,
    /// The moment the report is made (decides which claims are live).
    pub now: i64,
}

/// The kind of room a name makes: a direct message, a room with a label above the lowest, or an ordinary one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomClass {
    Dm,
    Private,
    Ordinary,
}

impl RoomClass {
    pub fn word(self) -> &'static str {
        match self {
            RoomClass::Dm => "dm",
            RoomClass::Private => "private",
            RoomClass::Ordinary => "open",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomCount {
    pub room: String,
    pub class: RoomClass,
    pub messages: i64,
    /// Messages under a sealed label: counted, never opened.
    pub sealed: i64,
    pub senders: i64,
    pub first_ts: i64,
    pub last_ts: i64,
}

pub type Counts = Vec<(String, i64)>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AskCounts {
    pub created: i64,
    pub answered: i64,
    pub closed: i64,
    pub with_scope: i64,
    pub with_refs: i64,
    pub ref_kinds: Counts,
    /// Seconds from the ask to its first answer by someone else, median and 90th percentile; none with no answers.
    pub first_answer_median: Option<i64>,
    pub first_answer_p90: Option<i64>,
    /// Asks in a sealed label are not read, so they are not in the counts above; how many messages were skipped.
    pub sealed_unread: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaimCounts {
    pub live: Counts,
    pub notes: Counts,
    /// Members waiting on a held claim now: the only conflict record the store keeps.
    pub waiting: Counts,
    pub yields: i64,
    /// What the claim event record (#1386) holds: counts by class and word, never a name.
    pub history: ClaimHistory,
}

/// The claim event record in the window (#1386): grants, refusals by rule word, releases and expiries by class, how
/// conflicts ended and how long claims were held. A renewal is not an event. Empty for a store that predates the
/// record or has no events yet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaimHistory {
    pub granted: Counts,
    pub refused: Counts,
    pub released: Counts,
    /// Releases by who released: `holder`, `session` (the session ended), `anchor` (the work item was reported done),
    /// `removed` (the member was removed).
    pub released_by: Counts,
    /// Leases that lapsed, by class (a holder's lease ran out).
    pub expired: Counts,
    /// Grants made over a claim whose lease had just lapsed under another member.
    pub takeovers: i64,
    /// Conflicts handled: a member refused and later granted a claim overlapping the one it was refused.
    pub handled: i64,
    /// Median seconds from the first refusal to the grant, over the handled conflicts.
    pub handled_wait_median: Option<i64>,
    /// Median seconds a claim was held until it was released or its lease lapsed, and how many holds that covers.
    pub hold_median: Option<i64>,
    pub holds: i64,
}

/// The machine checkpoints written in the window (#1408): how many, how many of those were a session's last, how many
/// replaced an earlier one, and their total size in bytes. Counts only; the checkpoint itself is never read here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MachineCheckpointCounts {
    pub written: i64,
    pub last: i64,
    pub replaced: i64,
    pub bytes: i64,
}

/// The passive claim events in the window (#1401): claims a hook took when a member edited a file, how many of those
/// renewed one the member already held, how many another member's claim refused, how many were released at a Stop or
/// SessionEnd, how many lapsed first and how many steps failed. Counts only: no claim name and no member.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassiveClaimCounts {
    pub granted: i64,
    pub renewed: i64,
    pub refused: i64,
    pub released: i64,
    pub expired: i64,
    pub errors: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemberCounts {
    /// Distinct member names that joined any room in scope.
    pub joined: i64,
    pub never_posted: i64,
    pub marked_done: i64,
    pub muted: i64,
    pub with_parent: i64,
    /// Distinct member names that posted (a live message), the base of the two counts below (#1411).
    pub posted: i64,
    /// Of those, the names whose read cursor never moved in any room in scope: the cursor `read` and `ack` move.
    pub never_read: i64,
    /// Of those, the names that were also never shown anything: no hook digest, wait or wake either.
    pub never_shown: i64,
    /// The `never_read` names by the UTC day of their first post, oldest first. Counts only, never a name.
    pub never_read_days: Counts,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageStats {
    pub schema: i64,
    pub room: Option<String>,
    pub since: Option<i64>,
    pub now: i64,
    pub rooms: Vec<RoomCount>,
    pub days: Vec<(String, i64)>,
    pub room_days: Vec<(String, String, i64)>,
    pub senders_distinct: i64,
    pub senders_top: Counts,
    pub attribution: Counts,
    pub verified_senders: i64,
    pub kinds: Counts,
    pub asks: AskCounts,
    pub claims: ClaimCounts,
    pub machine_checkpoints: MachineCheckpointCounts,
    pub passive_claims: PassiveClaimCounts,
    pub refusals: Counts,
    pub members: MemberCounts,
    pub hooks: Counts,
    /// Bytes the hooks put into session contexts (#1406), by event: the total and the largest single run, and the
    /// sessions that took the most. Sizes only; empty for a store from before the byte columns.
    pub hook_bytes: Counts,
    pub hook_bytes_max: Counts,
    pub sessions_top_bytes: Counts,
    pub audit: Counts,
    pub credentials_issued: i64,
    pub credentials_revoked: i64,
}

/// The class of a claim from its name alone: `ticket:` and `ask:` by prefix, a name with no colon is a file, anything
/// else (`merge:main`, a custom word) is other. The reason and the note are never looked at.
pub fn claim_class(name: &str) -> &'static str {
    if name.starts_with("ticket:") {
        "ticket"
    } else if name.starts_with("ask:") {
        "ask"
    } else if !name.contains(':') {
        "file"
    } else {
        "other"
    }
}

/// A word from a system vocabulary as it is shown: short and made of name characters, else `(other)`, so a stray
/// free-text value in a column that should hold a word can never print.
pub fn vocab(word: &str) -> String {
    let ok = !word.is_empty()
        && word.len() <= 32
        && word
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'));
    if ok {
        word.to_string()
    } else {
        "(other)".into()
    }
}

/// The median and the 90th percentile (nearest rank) of a list of seconds.
pub fn median_and_p90(mut secs: Vec<i64>) -> (Option<i64>, Option<i64>) {
    if secs.is_empty() {
        return (None, None);
    }
    secs.sort_unstable();
    let n = secs.len();
    let median = if n % 2 == 1 {
        secs[n / 2]
    } else {
        (secs[n / 2 - 1] + secs[n / 2]) / 2
    };
    let rank = (n * 9).div_ceil(10).max(1);
    (Some(median), Some(secs[rank - 1]))
}

/// `YYYY-MM-DD` (UTC) of a Unix second.
pub fn day_of(ts: i64) -> String {
    let z = ts.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// The first second of a `YYYY-MM-DD` day (UTC), for `--since`.
pub fn start_of_day(date: &str) -> Option<i64> {
    let ok = date.len() == 10
        && date
            .bytes()
            .enumerate()
            .all(|(i, b)| matches!(i, 4 | 7) == (b == b'-') && (b == b'-' || b.is_ascii_digit()));
    if !ok {
        return None;
    }
    super::asks::parse_utc(&format!("{date} 00:00"))
}

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

impl UsageStats {
    /// The report as stable plain text: fixed section order, sorted rows, no timestamp that varies between runs of the
    /// same store except the stated moment.
    pub fn to_text(&self) -> String {
        let mut o = String::new();
        o.push_str("klatch stats (counts only; no message text, ask text, claim detail, label or payload)\n");
        o.push_str(&format!(
            "schema {}; room {}; since {}; at {}\n",
            self.schema,
            self.room.as_deref().unwrap_or("all"),
            self.since.map_or("the start".into(), day_of),
            day_of(self.now)
        ));

        o.push_str("\nmessages per room\n");
        if self.rooms.is_empty() {
            o.push_str("  (none)\n");
        } else {
            let w = self
                .rooms
                .iter()
                .map(|r| r.room.chars().count())
                .max()
                .unwrap_or(4)
                .max(4);
            o.push_str(&format!(
                "  {:<w$}  {:<7}  {:>8}  {:>6}  {:>7}  {:<10}  {}\n",
                "room", "class", "messages", "sealed", "senders", "first", "last"
            ));
            for r in &self.rooms {
                o.push_str(&format!(
                    "  {:<w$}  {:<7}  {:>8}  {:>6}  {:>7}  {:<10}  {}\n",
                    r.room,
                    r.class.word(),
                    r.messages,
                    r.sealed,
                    r.senders,
                    day_of(r.first_ts),
                    day_of(r.last_ts)
                ));
            }
        }
        table(
            &mut o,
            "messages per day (UTC)",
            ["day", "messages"],
            &self.days,
        );

        o.push_str(&format!(
            "\nsenders: {} distinct, {} with a verified message\n",
            self.senders_distinct, self.verified_senders
        ));
        table(
            &mut o,
            "top senders",
            ["sender", "messages"],
            &self.senders_top,
        );
        table(
            &mut o,
            "attribution",
            ["attribution", "messages"],
            &self.attribution,
        );
        table(&mut o, "message kinds", ["kind", "messages"], &self.kinds);

        let a = &self.asks;
        o.push_str(&format!(
            "\nasks: {} created, {} answered, {} closed, {} with a declared scope, {} with refs\n",
            a.created, a.answered, a.closed, a.with_scope, a.with_refs
        ));
        o.push_str(&format!(
            "time to first answer: median {}, p90 {}\n",
            opt(a.first_answer_median),
            opt(a.first_answer_p90)
        ));
        if a.sealed_unread > 0 {
            o.push_str(&format!(
                "{} messages under a sealed label were not read for asks\n",
                a.sealed_unread
            ));
        }
        table(&mut o, "ask refs by kind", ["kind", "refs"], &a.ref_kinds);

        table(
            &mut o,
            "live claims by class",
            ["class", "claims"],
            &self.claims.live,
        );
        table(
            &mut o,
            "claim notes by class",
            ["class", "notes"],
            &self.claims.notes,
        );
        table(
            &mut o,
            "members waiting on a held claim now, by class",
            ["class", "waiting"],
            &self.claims.waiting,
        );
        o.push_str(&format!("claim yields on record: {}\n", self.claims.yields));
        let h = &self.claims.history;
        table(
            &mut o,
            "claim grants by class (new holds; renewals are not counted)",
            ["class", "grants"],
            &h.granted,
        );
        table(
            &mut o,
            "claim refusals by rule",
            ["rule", "refusals"],
            &h.refused,
        );
        table(
            &mut o,
            "claim releases by class",
            ["class", "releases"],
            &h.released,
        );
        table(
            &mut o,
            "claim releases by who released",
            ["by", "releases"],
            &h.released_by,
        );
        table(
            &mut o,
            "claim expiries by class (the holder's lease lapsed)",
            ["class", "expiries"],
            &h.expired,
        );
        o.push_str(&format!(
            "claims taken over after expiry: {}\nclaim conflicts handled (refused, then granted): {}, median wait {}\nclaim hold time: median {} over {} holds\n",
            h.takeovers,
            h.handled,
            opt(h.handled_wait_median),
            opt(h.hold_median),
            h.holds
        ));
        let mc = &self.machine_checkpoints;
        o.push_str(&format!(
            "machine checkpoints: {} written ({} at session end), {} replaced an earlier one, {} bytes\n",
            mc.written, mc.last, mc.replaced, mc.bytes
        ));
        let pc = &self.passive_claims;
        o.push_str(&format!(
            "passive claims: {} granted, {} renewed, {} refused, {} released, {} lapsed, {} failed\n",
            pc.granted, pc.renewed, pc.refused, pc.released, pc.expired, pc.errors
        ));
        table(
            &mut o,
            "refusals by rule",
            ["rule", "refusals"],
            &self.refusals,
        );

        let dm = self.rooms.iter().filter(|r| r.class == RoomClass::Dm);
        let (dm_rooms, dm_msgs) = dm.fold((0, 0), |(a, b), r| (a + 1, b + r.messages));
        let pv = self.rooms.iter().filter(|r| r.class == RoomClass::Private);
        let (pv_rooms, pv_msgs) = pv.fold((0, 0), |(a, b), r| (a + 1, b + r.messages));
        o.push_str(&format!(
            "\ndirect message rooms: {dm_rooms} ({dm_msgs} messages); private rooms: {pv_rooms} ({pv_msgs} messages)\n"
        ));

        let m = &self.members;
        o.push_str(&format!(
            "members: {} joined, {} never posted, {} marked done, {} muted, {} with a parent\n",
            m.joined, m.never_posted, m.marked_done, m.muted, m.with_parent
        ));
        o.push_str(&format!(
            "members who posted and never read: {} of {} ({} never shown anything by a hook, wait or wake)\n",
            m.never_read, m.posted, m.never_shown
        ));
        if !m.never_read_days.is_empty() {
            let days: Vec<String> = m
                .never_read_days
                .iter()
                .map(|(d, n)| format!("{d} {n}"))
                .collect();
            o.push_str(&format!("  by day of first post: {}\n", days.join(", ")));
        }
        table(
            &mut o,
            "hook runs by event (cumulative)",
            ["event", "runs"],
            &self.hooks,
        );
        o.push_str(
            "\nbytes injected into sessions by hook event (sizes only; the average is per run)\n",
        );
        if self.hook_bytes.is_empty() {
            o.push_str("  (none recorded)\n");
        }
        for (event, total) in &self.hook_bytes {
            let runs = self
                .hooks
                .iter()
                .find(|(e, _)| e == event)
                .map_or(0, |(_, n)| *n);
            let max = self
                .hook_bytes_max
                .iter()
                .find(|(e, _)| e == event)
                .map_or(0, |(_, n)| *n);
            let avg = if runs > 0 { total / runs } else { 0 };
            o.push_str(&format!(
                "  {event}: {total} bytes in {runs} runs (average {avg}, largest {max})\n"
            ));
        }
        table(
            &mut o,
            "largest sessions by bytes injected",
            ["session", "bytes"],
            &self.sessions_top_bytes,
        );
        table(&mut o, "admin audit by act", ["act", "count"], &self.audit);
        o.push_str(&format!(
            "\nworker credentials: {} issued, {} revoked\n",
            self.credentials_issued, self.credentials_revoked
        ));
        o
    }

    /// The report as one JSON object, counts and timestamps only, for the benchmark ledger.
    #[cfg(feature = "client")]
    pub fn to_json(&self) -> Value {
        let pairs = |c: &Counts| -> Value {
            Value::Array(
                c.iter()
                    .map(|(k, n)| json!({"name": k, "count": n}))
                    .collect(),
            )
        };
        json!({
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
            "asks": {
                "created": self.asks.created, "answered": self.asks.answered, "closed": self.asks.closed,
                "with_scope": self.asks.with_scope, "with_refs": self.asks.with_refs,
                "ref_kinds": pairs(&self.asks.ref_kinds),
                "first_answer_median_secs": self.asks.first_answer_median,
                "first_answer_p90_secs": self.asks.first_answer_p90,
                "sealed_unread": self.asks.sealed_unread,
            },
            "claims": {
                "live": pairs(&self.claims.live), "notes": pairs(&self.claims.notes),
                "waiting": pairs(&self.claims.waiting), "yields": self.claims.yields,
                "history": {
                    "granted": pairs(&self.claims.history.granted),
                    "refused": pairs(&self.claims.history.refused),
                    "released": pairs(&self.claims.history.released),
                    "released_by": pairs(&self.claims.history.released_by),
                    "expired": pairs(&self.claims.history.expired),
                    "takeovers": self.claims.history.takeovers,
                    "conflicts_handled": self.claims.history.handled,
                    "conflict_wait_median_secs": self.claims.history.handled_wait_median,
                    "hold_median_secs": self.claims.history.hold_median,
                    "holds": self.claims.history.holds,
                },
            },
            "machine_checkpoints": {
                "written": self.machine_checkpoints.written, "last": self.machine_checkpoints.last,
                "replaced": self.machine_checkpoints.replaced, "bytes": self.machine_checkpoints.bytes,
            },
            "passive_claims": {
                "granted": self.passive_claims.granted, "renewed": self.passive_claims.renewed,
                "refused": self.passive_claims.refused, "released": self.passive_claims.released,
                "lapsed": self.passive_claims.expired, "failed": self.passive_claims.errors,
            },
            "refusals": pairs(&self.refusals),
            "members": {
                "joined": self.members.joined, "never_posted": self.members.never_posted,
                "marked_done": self.members.marked_done, "muted": self.members.muted,
                "with_parent": self.members.with_parent,
                "posted": self.members.posted, "posted_never_read": self.members.never_read,
                "posted_never_shown": self.members.never_shown,
                "posted_never_read_by_day": pairs(&self.members.never_read_days),
            },
            "hooks": pairs(&self.hooks),
            "hook_bytes": pairs(&self.hook_bytes),
            "hook_bytes_max": pairs(&self.hook_bytes_max),
            "sessions_top_bytes": pairs(&self.sessions_top_bytes),
            "audit": pairs(&self.audit),
            "credentials": {"issued": self.credentials_issued, "revoked": self.credentials_revoked},
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_are_classed_by_name_prefix_only() {
        assert_eq!(claim_class("src/main.rs"), "file");
        assert_eq!(claim_class("ticket:12"), "ticket");
        assert_eq!(claim_class("ask:3"), "ask");
        assert_eq!(claim_class("merge:main"), "other");
        assert_eq!(claim_class("deploy:prod"), "other");
    }

    #[test]
    fn a_vocabulary_word_that_is_free_text_never_prints() {
        assert_eq!(vocab("PostToolUse"), "PostToolUse");
        assert_eq!(vocab("rate-limit"), "rate-limit");
        assert_eq!(vocab("secret plan for the merger"), "(other)");
        assert_eq!(vocab(""), "(other)");
    }

    #[test]
    fn median_and_p90_are_nearest_rank() {
        assert_eq!(median_and_p90(vec![]), (None, None));
        assert_eq!(median_and_p90(vec![5]), (Some(5), Some(5)));
        assert_eq!(median_and_p90(vec![10, 30, 20, 40]), (Some(25), Some(40)));
        let ten: Vec<i64> = (1..=10).collect();
        assert_eq!(median_and_p90(ten), (Some(5), Some(9)));
    }

    #[test]
    fn days_round_trip() {
        assert_eq!(day_of(0), "1970-01-01");
        assert_eq!(day_of(1_795_392_000), "2026-11-23");
        assert_eq!(start_of_day("2026-11-23"), Some(1_795_392_000));
        assert_eq!(start_of_day("2026-13-01"), None);
        assert_eq!(start_of_day("yesterday"), None);
        assert_eq!(start_of_day("2026-1-1"), None);
    }
}
