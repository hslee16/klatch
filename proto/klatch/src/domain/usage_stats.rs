//! The counts-only usage report behind `klatch admin stats` (#1377): the shape of the report, how claims are
//! classified, and how it is written as plain text and as JSON. This file holds no store access and no message text:
//! every field is a count, a timestamp, a room or member name, or a word from a closed system vocabulary (a rule, an
//! event, an admin act, a message kind) that `vocab` has checked. Nothing a member wrote (a body, an ask, a claim
//! reason or note, a label, a payload) can reach it.

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

/// The machine status updates written in the window (#1403), in all and by class. Counts only: the table behind it
/// holds a time, a room and a class word, never a tool, a command or a member.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MachineStatusCounts {
    pub updates: i64,
    pub working: i64,
    pub idle: i64,
    pub parked: i64,
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
    pub machine_status: MachineStatusCounts,
    pub passive_claims: PassiveClaimCounts,
    pub ask_routing: crate::domain::RoutingCounts,
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

mod render;

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
