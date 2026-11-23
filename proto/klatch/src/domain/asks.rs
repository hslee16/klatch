//! Asks: lobby messages that start with `seeking:`. klatch derives their state from the messages, the live
//! claims (DESIGN section 25.4). An ask is open until someone answers it or its asker closes it. Releasing an
//! `ask:<seq>` claim does not end it: a helper that gives up releases too.

use crate::domain::{Claim, Message, valid_name};

/// A message whose body starts with this text is an ask.
pub const ASK_PREFIX: &str = "seeking:";

/// How long an open ask may go without a claim before its asker is told.
pub const STALE_AFTER_SECS: i64 = 60;

/// How many distinct helpers must decline an ask as `cannot`, with none busy or failed, before klatch tells
/// the asker that nobody here may be able to do it.
pub const NO_CAPABLE_AFTER: usize = 2;

/// The lease an `ask:<N>` claim gets when none is asked for, and the longest it may be: short, so a helper that
/// fails or disappears frees the ask in about a minute; a working helper renews it by claiming again (DESIGN
/// section 28).
pub const ASK_LEASE_DEFAULT_SECS: i64 = 90;
pub const ASK_LEASE_MAX_SECS: i64 = 300;
/// The lease of any other claim when none is asked for.
pub const CLAIM_DEFAULT_SECS: i64 = 600;
/// The longest a claim may last: 24 hours (DESIGN D5). A longer request is refused, not shortened, so the claimant
/// learns the limit; a claim that must last longer is renewed by claiming again.
pub const CLAIM_MAX_SECS: i64 = 86_400;
/// A working room with news this recently keeps its ask from being reported stale.
pub const ROOM_ACTIVE_SECS: i64 = 120;

/// The lease to use for a claim: the one asked for, or the default for the path; an `ask:` claim never longer
/// than `ASK_LEASE_MAX_SECS`.
pub fn claim_ttl(path: &str, requested: Option<i64>) -> i64 {
    if crate::domain::is_merge_claim(path) {
        return requested
            .unwrap_or(CLAIM_DEFAULT_SECS)
            .min(crate::domain::MERGE_CLAIM_MAX_SECS);
    }
    if path.starts_with("ask:") {
        requested
            .unwrap_or(ASK_LEASE_DEFAULT_SECS)
            .min(ASK_LEASE_MAX_SECS)
    } else {
        requested.unwrap_or(CLAIM_DEFAULT_SECS)
    }
}

/// A working room for an ask, named `ask-<N>` (N is the ask's number): anyone can join it, the helper can bring in
/// others and the asker can invite several members at once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AskRoom {
    pub name: String,
    pub members: usize,
    /// Unix seconds of the room's latest activity.
    pub last_activity: i64,
}

/// A helper's question to the asker, still unanswered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clarification {
    pub by: String,
    pub question: String,
}

/// The ask number and text in `need-info #<N>: <question>` (a helper's question) or `info #<N>: <details>` (the
/// asker's answer).
pub fn parse_clarification(body: &str) -> Option<(bool, i64, String)> {
    let (need, rest) = if let Some(r) = body.strip_prefix("need-info #") {
        (true, r)
    } else {
        (false, body.strip_prefix("info #")?)
    };
    let (n, after) = number(rest)?;
    Some((need, n, after.strip_prefix(':')?.trim().to_string()))
}

/// Why a helper did not take an ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeKind {
    /// `declined #N busy: ...`: capable, but not now. The ask stays open.
    Busy,
    /// `declined #N cannot: ...`: not capable, or not mine. The ask stays open for others.
    Cannot,
    /// `failed #N: ...`: tried and did not succeed. The ask stays open.
    Failed,
    /// `partial #N: ...`: what a helper got done before it stopped, for the next helper to start from.
    Partial,
}

/// A helper's decline or failure on an ask, with its stated reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub kind: OutcomeKind,
    pub by: String,
    pub reason: String,
}

/// An ask nobody has answered or closed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenAsk {
    pub seq: i64,
    pub sender: String,
    pub text: String,
    pub ts: i64,
    /// A live claim on `ask:<seq>` exists.
    pub claimed: bool,
    /// Declines and failures by helpers other than the asker, oldest first.
    pub outcomes: Vec<Outcome>,
    /// A helper's question the asker has not answered yet: the ask is waiting on the asker.
    pub waiting_on_asker: Option<Clarification>,
    /// The ask's working room, if anyone has joined `ask-<N>`.
    pub room: Option<AskRoom>,
    /// The dm the exchange moved to (`moved #N: dm with NAME`, #1188), with the dm room once the store has found it.
    pub moved: Option<MovedDm>,
    /// The scope the asker declared on the ask (#1171), if any. The fold of messages leaves it `None`; the store
    /// fills it from the ask's scope record.
    pub scope: Option<String>,
    /// The references the asker attached (#1318); the store fills them from the ask's record. Empty when none.
    pub refs: Vec<crate::domain::AskRef>,
}

impl MovedDm {
    /// The member of the pair that is not the asker: who the asker's exchange is with.
    pub fn counterpart(&self, asker: &str) -> &str {
        if self.by == asker {
            &self.with
        } else {
            &self.by
        }
    }
}

/// Where a moved ask's exchange went: the member who posted the pointer, the member named, and their dm room.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MovedDm {
    pub by: String,
    pub with: String,
    pub room: Option<AskRoom>,
}

impl OpenAsk {
    /// How many distinct helpers gave this outcome.
    pub fn distinct(&self, kind: OutcomeKind) -> usize {
        let mut who: Vec<&str> = self
            .outcomes
            .iter()
            .filter(|o| o.kind == kind)
            .map(|o| o.by.as_str())
            .collect();
        who.sort_unstable();
        who.dedup();
        who.len()
    }

    fn outcome_parts(&self) -> Vec<String> {
        [
            ("busy", OutcomeKind::Busy),
            ("cannot", OutcomeKind::Cannot),
            ("failed", OutcomeKind::Failed),
            ("partial", OutcomeKind::Partial),
        ]
        .iter()
        .filter_map(|(name, k)| {
            let n = self.distinct(*k);
            (n > 0).then(|| format!("{name} {n}"))
        })
        .collect()
    }

    /// How helpers answered the ask without taking it, for display: `[busy 1, cannot 2]`, or empty.
    pub fn outcome_summary(&self) -> String {
        bracket(self.outcome_parts())
    }

    /// Everything to know about the ask at a glance, for display: the scope the asker declared, how helpers answered
    /// it, whether it waits on the asker, and its working room: `[scope git:foo, busy 1, needs info, room ask-7 3
    /// members]`, or empty.
    pub fn summary(&self, now: i64) -> String {
        let mut parts = Vec::new();
        if let Some(scope) = &self.scope {
            // a scope is 1 to 80 characters of letters, digits and . _ : - /, so it is safe to print as it is (#1171)
            parts.push(format!("scope {scope}"));
        }
        parts.extend(self.outcome_parts());
        if let Some(d) = self.deadline() {
            parts.push(deadline_phrase(d, now));
        }
        if self.waiting_on_asker.is_some() {
            parts.push("needs info".into());
        }
        if let Some(r) = &self.room {
            parts.push(format!("room {} {} members", r.name, r.members));
        }
        if let Some(m) = self.moved.as_ref().filter(|m| m.room.is_some()) {
            // the name only: nothing of what is said in the dm
            parts.push(format!("in a dm with {}", m.counterpart(&self.sender)));
        }
        bracket(parts)
    }

    /// The ask's working room has had news recently, so the ask is being worked on.
    pub fn room_active(&self, now: i64) -> bool {
        let recent = |r: &AskRoom| now - r.last_activity <= ROOM_ACTIVE_SECS;
        self.room.as_ref().is_some_and(recent)
            || self
                .moved
                .as_ref()
                .and_then(|m| m.room.as_ref())
                .is_some_and(recent)
    }

    /// When the asker needs this by, if the text says `(by YYYY-MM-DD)` or `(by YYYY-MM-DD HH:MM)` (UTC).
    pub fn deadline(&self) -> Option<i64> {
        ask_deadline(&self.text)
    }

    /// How long this ask may go unclaimed before its asker is told: the room default, or until its deadline
    /// when it names one later than that. A long task needs no hurry, and a deadline in the past of the ask never
    /// lowers the default (#306).
    pub fn stale_after(&self, default: i64) -> i64 {
        self.deadline()
            .map_or(default, |d| (d - self.ts).max(default))
    }

    /// Enough helpers said `cannot` and none said `busy` or `failed`: nobody here may be able to do this.
    pub fn no_capable_member(&self) -> bool {
        self.distinct(OutcomeKind::Cannot) >= NO_CAPABLE_AFTER
            && self.distinct(OutcomeKind::Busy) == 0
            && self.distinct(OutcomeKind::Failed) == 0
            && self.distinct(OutcomeKind::Partial) == 0
    }
}

/// `due in 3d` or `overdue 2h`, in the largest whole unit (days, hours, minutes).
fn deadline_phrase(deadline: i64, now: i64) -> String {
    let left = deadline - now;
    let n = left.abs();
    let (v, unit) = if n >= 86_400 {
        (n / 86_400, "d")
    } else if n >= 3600 {
        (n / 3600, "h")
    } else {
        (n / 60, "m")
    };
    if left >= 0 {
        format!("due in {v}{unit}")
    } else {
        format!("overdue {v}{unit}")
    }
}

/// The last `(by ...)` deadline in an ask's text, as Unix seconds. A bare date means the end of that day.
pub fn ask_deadline(text: &str) -> Option<i64> {
    let start = text.rfind("(by ")? + 4;
    let end = start + text[start..].find(')')?;
    parse_utc(&text[start..end])
}

/// A UTC moment written `YYYY-MM-DD` (the end of that day) or `YYYY-MM-DD HH:MM`, as Unix seconds.
pub(crate) fn parse_utc(s: &str) -> Option<i64> {
    let (date, time) = match s.split_once(' ') {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let secs_in_day = match time {
        Some(t) => {
            let (h, m) = t.split_once(':')?;
            let (h, m): (i64, i64) = (h.parse().ok()?, m.parse().ok()?);
            if h > 23 || m > 59 {
                return None;
            }
            h * 3600 + m * 60
        }
        None => 86_399,
    };
    Some(civil_days(date)? * 86_400 + secs_in_day)
}

/// The days in month `m` (1 to 12) of year `y`.
fn days_in_month(y: i64, m: i64) -> i64 {
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    match m {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn is_real_day(y: i64, m: i64, d: i64) -> bool {
    (1..=12).contains(&m) && (1..=days_in_month(y, m)).contains(&d)
}

/// The three numbers of a `Y-M-D` date, and none when there are not exactly three.
fn ymd(date: &str) -> Option<(i64, i64, i64)> {
    let mut parts = date.split('-');
    let y = parts.next()?.parse().ok()?;
    let m = parts.next()?.parse().ok()?;
    let d = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((y, m, d))
}

/// Days since 1970-01-01 of a `YYYY-MM-DD` date, or none when it is not a real calendar day.
fn civil_days(date: &str) -> Option<i64> {
    let (y, m, d) = ymd(date)?;
    if !is_real_day(y, m, d) {
        return None;
    }
    // days from civil, after Howard Hinnant
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

fn bracket(parts: Vec<String>) -> String {
    if parts.is_empty() {
        String::new()
    } else {
        format!("[{}]", parts.join(", "))
    }
}

/// A decline or failure message: its kind, the ask number, and the reason.
pub fn parse_outcome(body: &str) -> Option<(OutcomeKind, i64, String)> {
    if let Some(rest) = body.strip_prefix("partial #") {
        let (n, after) = number(rest)?;
        return Some((
            OutcomeKind::Partial,
            n,
            after.strip_prefix(':')?.trim().to_string(),
        ));
    }
    let reason = |rest: &str| rest.trim().to_string();
    if let Some(rest) = body.strip_prefix("declined #") {
        let (n, after) = number(rest)?;
        let after = after.strip_prefix(' ')?;
        let (kind, rest) = if let Some(r) = after.strip_prefix("busy:") {
            (OutcomeKind::Busy, r)
        } else {
            (OutcomeKind::Cannot, after.strip_prefix("cannot:")?)
        };
        return Some((kind, n, reason(rest)));
    }
    let (n, after) = number(body.strip_prefix("failed #")?)?;
    Some((OutcomeKind::Failed, n, reason(after.strip_prefix(':')?)))
}

/// The prefixes of every word that `open_asks` folds (the ask, its answer, close, reopen, decline, failure, partial
/// the clarification pair and the pointer to a dm), for a store query that reads only those messages at any depth (#731).
pub const ASK_WORDS: [&str; 10] = [
    ASK_PREFIX,
    "answer to #",
    "closed #",
    "reopen #",
    "partial #",
    "declined #",
    "failed #",
    "need-info #",
    "info #",
    "moved #",
];

pub fn is_ask(body: &str) -> bool {
    body.starts_with(ASK_PREFIX)
}

/// The number after `#` at the start of `rest`, if it has digits.
fn number(rest: &str) -> Option<(i64, &str)> {
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    rest[..end].parse().ok().map(|n| (n, &rest[end..]))
}

/// The ask number in `answer to #<N>:` at the start of a body.
pub fn answer_target(body: &str) -> Option<i64> {
    let (n, after) = number(body.strip_prefix("answer to #")?)?;
    after.starts_with(':').then_some(n)
}

/// The ask number in `closed #<N>` at the start of a body.
pub fn closed_target(body: &str) -> Option<i64> {
    number(body.strip_prefix("closed #")?).map(|(n, _)| n)
}

/// How the asker closed an ask (#232): a bare `closed #N` says nothing about the result, `closed #N ok` says the
/// answer served, `closed #N unresolved` says the ask was given up without a usable answer. Every one ends the ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseOutcome {
    Unspecified,
    Ok,
    Unresolved,
}

impl CloseOutcome {
    /// The word shown for it, `closed` for a bare close.
    pub fn word(self) -> &'static str {
        match self {
            CloseOutcome::Unspecified => "closed",
            CloseOutcome::Ok => "closed ok",
            CloseOutcome::Unresolved => "closed unresolved",
        }
    }
}

/// The outcome in a `closed #<N>` body: the first word after the number, lower-cased, without trailing
/// punctuation. Any other word, or none, is `Unspecified` and still a close.
pub fn close_outcome(body: &str) -> Option<CloseOutcome> {
    let (_, after) = number(body.strip_prefix("closed #")?)?;
    if !after.starts_with(char::is_whitespace) {
        return Some(CloseOutcome::Unspecified);
    }
    let word = after
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches(|c: char| c.is_ascii_punctuation())
        .to_ascii_lowercase();
    Some(match word.as_str() {
        "ok" => CloseOutcome::Ok,
        "unresolved" => CloseOutcome::Unresolved,
        _ => CloseOutcome::Unspecified,
    })
}

/// How the asker last closed `ask` in `messages`, if a close of theirs is newer than their latest reopen.
pub fn latest_close(messages: &[Message], ask: &Message) -> Option<CloseOutcome> {
    let latest_reopen = messages
        .iter()
        .filter(|m| m.sender == ask.sender && reopen_target(&m.body) == Some(ask.seq))
        .map(|m| m.seq)
        .max();
    messages
        .iter()
        .filter(|m| m.sender == ask.sender && closed_target(&m.body) == Some(ask.seq))
        .filter(|m| latest_reopen.is_none_or(|r| m.seq > r))
        .max_by_key(|m| m.seq)
        .and_then(|m| close_outcome(&m.body))
}

/// The ask number in `reopen #<N>:` at the start of a body.
pub fn reopen_target(body: &str) -> Option<i64> {
    let (n, after) = number(body.strip_prefix("reopen #")?)?;
    after.starts_with(':').then_some(n)
}

/// The member name in `moved #<N>: dm with <NAME>` at the start of a body, with the ask number (#1188).
/// Only the number and the name are read; any text after the name is data and is never kept.
pub fn moved_target(body: &str) -> Option<(i64, String)> {
    let (n, after) = number(body.strip_prefix("moved #")?)?;
    let name = after.strip_prefix(": dm with ")?;
    let token = name.split_whitespace().next()?;
    // punctuation that ends the sentence is not part of the name
    let token = token.trim_end_matches(['.', ',', ';', ':', '!', '?']);
    valid_name(token).then(|| (n, token.to_string()))
}

/// The latest `moved #<seq>:` that points at a dm the asker is in: posted after the ask by the asker, or by anyone
/// naming the asker, so a third pair cannot keep someone else's ask alive (#1188).
fn moved_to(messages: &[Message], ask: &Message) -> Option<MovedDm> {
    messages
        .iter()
        .filter(|m| m.seq > ask.seq)
        .filter_map(|m| {
            let (n, with) = moved_target(&m.body)?;
            let involves = m.sender == ask.sender || with == ask.sender;
            (n == ask.seq && m.sender != with && involves).then(|| (m.seq, m.sender.clone(), with))
        })
        .max_by_key(|(seq, _, _)| *seq)
        .map(|(_, by, with)| MovedDm {
            by,
            with,
            room: None,
        })
}

/// The asks in `messages` that are still open. `live_claims` are unexpired claims.
pub fn open_asks(messages: &[Message], live_claims: &[Claim]) -> Vec<OpenAsk> {
    let mut asks: Vec<&Message> = messages.iter().filter(|m| is_ask(&m.body)).collect();
    asks.sort_by_key(|m| m.seq);
    asks.into_iter()
        .filter(|a| {
            // An answer by someone else or a close by the asker ends the ask; a `reopen` by the asker after the
            // latest of those (#232) puts it back. Only the asker's reopen counts.
            let latest = |ends: bool| {
                messages
                    .iter()
                    .filter(|m| {
                        if ends {
                            (m.sender != a.sender && answer_target(&m.body) == Some(a.seq))
                                || (m.sender == a.sender && closed_target(&m.body) == Some(a.seq))
                        } else {
                            m.sender == a.sender && reopen_target(&m.body) == Some(a.seq)
                        }
                    })
                    .map(|m| m.seq)
                    .max()
            };
            match (latest(true), latest(false)) {
                (None, _) => true,
                (Some(end), Some(reopen)) => reopen > end,
                (Some(_), None) => false,
            }
        })
        .map(|a| OpenAsk {
            seq: a.seq,
            sender: a.sender.clone(),
            text: a.body.clone(),
            ts: a.ts,
            claimed: live_claims
                .iter()
                .any(|c| c.path == format!("ask:{}", a.seq)),
            outcomes: messages
                .iter()
                .filter(|m| m.sender != a.sender)
                .filter_map(|m| {
                    let (kind, n, why) = parse_outcome(&m.body)?;
                    (n == a.seq).then(|| Outcome {
                        kind,
                        by: m.sender.clone(),
                        reason: why,
                    })
                })
                .collect(),
            waiting_on_asker: waiting_on(messages, a),
            room: None,
            moved: moved_to(messages, a),
            scope: None,
            refs: Vec::new(),
        })
        .collect()
}

/// The helper's question to the asker that the asker has not answered yet, if any: the latest `need-info` from
/// someone else, unless the asker has answered with `info` after it.
fn waiting_on(messages: &[Message], ask: &Message) -> Option<Clarification> {
    let mut ordered: Vec<&Message> = messages.iter().collect();
    ordered.sort_by_key(|m| m.seq);
    let mut waiting = None;
    for m in ordered {
        let Some((need, n, text)) = parse_clarification(&m.body) else {
            continue;
        };
        if n != ask.seq {
            continue;
        }
        if need && m.sender != ask.sender {
            waiting = Some(Clarification {
                by: m.sender.clone(),
                question: text,
            });
        } else if !need && m.sender == ask.sender {
            waiting = None;
        }
    }
    waiting
}

#[cfg(test)]
mod tests;
