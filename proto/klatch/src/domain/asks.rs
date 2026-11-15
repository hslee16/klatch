//! Asks: lobby messages that start with `seeking:`. klatch derives their state from the messages, the live
//! claims (DESIGN section 25.4). An ask is open until someone answers it or its asker closes it. Releasing an
//! `ask:<seq>` claim does not end it: a helper that gives up releases too.

use crate::domain::{Claim, Message};

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

    /// Everything to know about the ask at a glance, for display: how helpers answered it, whether it waits on the
    /// asker, and its working room: `[busy 1, needs info, room ask-7 3 members]`, or empty.
    pub fn summary(&self, now: i64) -> String {
        let mut parts = self.outcome_parts();
        if let Some(d) = self.deadline() {
            parts.push(deadline_phrase(d, now));
        }
        if self.waiting_on_asker.is_some() {
            parts.push("needs info".into());
        }
        if let Some(r) = &self.room {
            parts.push(format!("room {} {} members", r.name, r.members));
        }
        bracket(parts)
    }

    /// The ask's working room has had news recently, so the ask is being worked on.
    pub fn room_active(&self, now: i64) -> bool {
        self.room
            .as_ref()
            .is_some_and(|r| now - r.last_activity <= ROOM_ACTIVE_SECS)
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
    let (date, time) = match text[start..end].split_once(' ') {
        Some((d, t)) => (d, Some(t)),
        None => (&text[start..end], None),
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

/// Days since 1970-01-01 of a `YYYY-MM-DD` date, or none when it is not a real calendar day.
fn civil_days(date: &str) -> Option<i64> {
    let mut parts = date.split('-');
    let (y, m, d): (i64, i64, i64) = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if parts.next().is_some() || !(1..=12).contains(&m) || d < 1 || d > month_days[m as usize - 1] {
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

/// The asks in `messages` that are still open. `live_claims` are unexpired claims.
pub fn open_asks(messages: &[Message], live_claims: &[Claim]) -> Vec<OpenAsk> {
    let mut asks: Vec<&Message> = messages.iter().filter(|m| is_ask(&m.body)).collect();
    asks.sort_by_key(|m| m.seq);
    asks.into_iter()
        .filter(|a| {
            !messages.iter().any(|m| {
                (m.sender != a.sender && answer_target(&m.body) == Some(a.seq))
                    || (m.sender == a.sender && closed_target(&m.body) == Some(a.seq))
            })
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
mod tests {
    use super::*;
    use crate::domain::Attribution;

    fn msg(seq: i64, sender: &str, body: &str, ts: i64) -> Message {
        Message {
            id: format!("id{seq}"),
            seq,
            sender: sender.into(),
            attribution: Attribution::Unattributed,
            label: "public".into(),
            body: body.into(),
            ts,
            kind: None,
            payload: None,
        }
    }

    fn claim(path: &str) -> Claim {
        Claim {
            path: path.into(),
            owner: "helper".into(),
            reason: String::new(),
            expires: 1000,
            scope: String::new(),
        }
    }

    #[test]
    fn an_ask_starts_with_the_prefix() {
        let cases = [
            ("seeking: a thing", true),
            ("seeking:no space", true),
            ("not seeking: a thing", false),
            ("Seeking: a thing", false),
            (" seeking: leading space", false),
            ("", false),
        ];
        for (text, expected) in cases {
            assert_eq!(is_ask(text), expected, "{text:?}");
        }
    }

    #[test]
    fn answers_name_their_ask_by_number() {
        let cases = [
            ("answer to #3: here", Some(3)),
            ("answer to #12:x", Some(12)),
            ("answer to #3 here", None),
            ("answer to #: x", None),
            ("my answer to #3: x", None),
        ];
        for (text, expected) in cases {
            assert_eq!(answer_target(text), expected, "{text:?}");
        }
    }

    #[test]
    fn closes_name_their_ask_by_number() {
        let cases = [
            ("closed #3", Some(3)),
            ("closed #3 thanks", Some(3)),
            ("closed #3.", Some(3)),
            ("closed #x", None),
            ("not closed #3", None),
            ("closed #", None),
        ];
        for (text, expected) in cases {
            assert_eq!(closed_target(text), expected, "{text:?}");
        }
    }

    #[test]
    fn an_unanswered_ask_is_open_and_unclaimed() {
        let m = [msg(1, "asker", "seeking: slugify", 5)];
        let o = open_asks(&m, &[]);
        assert_eq!(o.len(), 1);
        assert_eq!(
            (o[0].seq, o[0].sender.as_str(), o[0].ts, o[0].claimed),
            (1, "asker", 5, false)
        );
        assert_eq!(o[0].text, "seeking: slugify");
    }

    #[test]
    fn a_live_claim_on_ask_n_marks_it_claimed() {
        let m = [msg(4, "asker", "seeking: x", 5)];
        assert!(open_asks(&m, &[claim("ask:4")])[0].claimed);
        assert!(!open_asks(&m, &[claim("ask:5")])[0].claimed);
        assert!(!open_asks(&m, &[claim("src")])[0].claimed);
    }

    #[test]
    fn a_reply_from_someone_else_ends_the_ask() {
        let m = [
            msg(1, "asker", "seeking: x", 5),
            msg(2, "helper", "answer to #1: here", 9),
        ];
        assert!(open_asks(&m, &[]).is_empty());
    }

    #[test]
    fn the_askers_own_answer_to_itself_does_not_end_the_ask() {
        let m = [
            msg(1, "asker", "seeking: x", 5),
            msg(2, "asker", "answer to #1: me", 9),
        ];
        assert_eq!(open_asks(&m, &[]).len(), 1);
    }

    #[test]
    fn the_asker_closing_ends_the_ask_but_someone_elses_close_does_not() {
        let closed = [
            msg(1, "asker", "seeking: x", 5),
            msg(2, "asker", "closed #1", 9),
        ];
        assert!(open_asks(&closed, &[]).is_empty());
        let other = [
            msg(1, "asker", "seeking: x", 5),
            msg(2, "stranger", "closed #1", 9),
        ];
        assert_eq!(open_asks(&other, &[]).len(), 1);
    }

    #[test]
    fn a_reply_only_ends_the_ask_it_names() {
        let m = [
            msg(1, "a", "seeking: one", 1),
            msg(2, "b", "seeking: two", 2),
            msg(3, "h", "answer to #2: ok", 3),
        ];
        let o = open_asks(&m, &[]);
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].seq, 1);
    }

    #[test]
    fn non_asks_are_ignored_and_order_is_by_seq() {
        let m = [
            msg(1, "a", "hello", 1),
            msg(2, "a", "seeking: one", 2),
            msg(3, "b", "chatter", 3),
            msg(4, "b", "seeking: two", 4),
        ];
        let seqs: Vec<_> = open_asks(&m, &[]).iter().map(|a| a.seq).collect();
        assert_eq!(seqs, [2, 4]);
    }

    #[test]
    fn outcomes_are_parsed_with_their_kind_ask_and_reason() {
        let cases = [
            (
                "declined #4 busy: on LIB-2",
                Some((OutcomeKind::Busy, 4, "on LIB-2")),
            ),
            (
                "declined #12 cannot: not my area",
                Some((OutcomeKind::Cannot, 12, "not my area")),
            ),
            (
                "failed #3: the build broke",
                Some((OutcomeKind::Failed, 3, "the build broke")),
            ),
            ("declined #4 busy:", Some((OutcomeKind::Busy, 4, ""))),
        ];
        for (text, expected) in cases {
            let expected = expected.map(|(k, n, r)| (k, n, r.to_string()));
            assert_eq!(parse_outcome(text), expected, "{text:?}");
        }
    }

    #[test]
    fn a_malformed_outcome_is_not_parsed() {
        let cases = [
            "declined #4: no class",
            "declined #x busy: r",
            "failed #3 the build broke",
            "I declined #4 busy: r",
        ];
        for text in cases {
            assert_eq!(parse_outcome(text), None, "{text:?}");
        }
    }

    fn with(kind: OutcomeKind, ask: i64, who: &str) -> Message {
        let word = match kind {
            OutcomeKind::Busy => format!("declined #{ask} busy: r"),
            OutcomeKind::Cannot => format!("declined #{ask} cannot: r"),
            OutcomeKind::Failed => format!("failed #{ask}: r"),
            OutcomeKind::Partial => format!("partial #{ask}: r"),
        };
        msg(10, who, &word, 9)
    }

    #[test]
    fn an_open_ask_carries_the_outcomes_other_helpers_gave_it() {
        let m = [
            msg(1, "asker", "seeking: x", 5),
            with(OutcomeKind::Busy, 1, "h1"),
            with(OutcomeKind::Cannot, 1, "h2"),
            with(OutcomeKind::Cannot, 2, "h3"),
            with(OutcomeKind::Failed, 1, "asker"),
        ];
        let o = open_asks(&m, &[]);
        assert_eq!(o[0].outcomes.len(), 2, "only other helpers, only this ask");
        assert_eq!(
            (o[0].outcomes[0].by.as_str(), o[0].outcomes[0].kind),
            ("h1", OutcomeKind::Busy)
        );
    }

    #[test]
    fn nobody_here_can_when_two_distinct_helpers_cannot_and_none_is_busy_or_failed() {
        let ask = |outs: &[(OutcomeKind, &str)]| {
            let mut m = vec![msg(1, "asker", "seeking: x", 5)];
            for (k, w) in outs {
                m.push(with(*k, 1, w));
            }
            open_asks(&m, &[]).remove(0)
        };
        let two = ask(&[(OutcomeKind::Cannot, "a"), (OutcomeKind::Cannot, "b")]);
        assert_eq!(two.distinct(OutcomeKind::Cannot), 2);
        assert!(two.no_capable_member());
        assert!(!ask(&[(OutcomeKind::Cannot, "a")]).no_capable_member());
        assert!(
            !ask(&[(OutcomeKind::Cannot, "a"), (OutcomeKind::Cannot, "a")]).no_capable_member(),
            "the same helper twice is one"
        );
        assert!(
            !ask(&[
                (OutcomeKind::Cannot, "a"),
                (OutcomeKind::Cannot, "b"),
                (OutcomeKind::Busy, "c")
            ])
            .no_capable_member()
        );
        assert!(
            !ask(&[
                (OutcomeKind::Cannot, "a"),
                (OutcomeKind::Cannot, "b"),
                (OutcomeKind::Failed, "c")
            ])
            .no_capable_member()
        );
    }

    #[test]
    fn the_lease_of_an_ask_claim_is_short_by_default_and_capped() {
        let cases = [
            (None, 90),
            (Some(30), 30),
            (Some(3600), 300), // an ask lease is capped
        ];
        for (asked, expected) in cases {
            assert_eq!(claim_ttl("ask:7", asked), expected, "{asked:?}");
        }
    }

    #[test]
    fn other_claims_keep_their_default_and_are_not_capped_here() {
        assert_eq!(claim_ttl("src/lib.rs", None), 600);
        assert_eq!(claim_ttl("src/lib.rs", Some(3600)), 3600);
    }

    #[test]
    fn a_partial_note_is_parsed_and_shown_and_counts_as_someone_capable() {
        assert_eq!(
            parse_outcome("partial #4: parsed the file, tests not written"),
            Some((
                OutcomeKind::Partial,
                4,
                "parsed the file, tests not written".into()
            ))
        );
        assert_eq!(parse_outcome("partial #4 no colon"), None);
        let mut m = vec![msg(1, "asker", "seeking: x", 5)];
        for (i, who) in ["h1", "h2"].iter().enumerate() {
            m.push(msg(10 + i as i64, who, "declined #1 cannot: no", 6));
        }
        m.push(msg(20, "h3", "partial #1: half done", 7));
        let a = open_asks(&m, &[]).remove(0);
        assert!(a.summary(10).contains("partial 1"), "{}", a.summary(10));
        assert!(
            !a.no_capable_member(),
            "someone worked on it, so someone here can"
        );
    }

    #[test]
    fn clarification_words_are_parsed_with_their_kind_ask_and_text() {
        let cases = [
            (
                "need-info #3: which format?",
                Some((true, 3, "which format?")),
            ),
            ("info #3: json", Some((false, 3, "json"))),
            ("info #3 json", None),
            ("need-info #x: q", None),
            ("my info #3: q", None),
        ];
        for (text, expected) in cases {
            let expected = expected.map(|(need, n, t)| (need, n, t.to_string()));
            assert_eq!(parse_clarification(text), expected, "{text:?}");
        }
    }

    #[test]
    fn an_ask_waits_on_its_asker_until_the_asker_answers_the_helpers_question() {
        let base = vec![
            msg(1, "asker", "seeking: x", 5),
            msg(2, "helper", "need-info #1: which format?", 6),
        ];
        let waiting = open_asks(&base, &[]).remove(0);
        let w = waiting.waiting_on_asker.expect("waiting on the asker");
        assert_eq!(
            (w.by.as_str(), w.question.as_str()),
            ("helper", "which format?")
        );
        let mut answered = base.clone();
        answered.push(msg(3, "asker", "info #1: json", 7));
        assert!(
            open_asks(&answered, &[])
                .remove(0)
                .waiting_on_asker
                .is_none()
        );
        let mut again = answered.clone();
        again.push(msg(4, "helper", "need-info #1: and the encoding?", 8));
        assert_eq!(
            open_asks(&again, &[])
                .remove(0)
                .waiting_on_asker
                .unwrap()
                .question,
            "and the encoding?"
        );
    }

    #[test]
    fn only_a_helper_can_ask_and_only_the_asker_can_answer() {
        let m = vec![
            msg(1, "asker", "seeking: x", 5),
            msg(2, "asker", "need-info #1: asking myself", 6),
            msg(3, "stranger", "info #1: not mine to say", 7),
        ];
        assert!(
            open_asks(&m, &[]).remove(0).waiting_on_asker.is_none(),
            "the asker cannot wait on itself"
        );
        let m2 = vec![
            msg(1, "asker", "seeking: x", 5),
            msg(2, "helper", "need-info #1: q", 6),
            msg(3, "stranger", "info #1: not the asker", 7),
        ];
        assert!(
            open_asks(&m2, &[]).remove(0).waiting_on_asker.is_some(),
            "a stranger's info does not answer it"
        );
    }

    #[test]
    fn the_summary_names_the_wait_and_the_working_room_and_the_room_counts_as_activity() {
        let mut a = open_asks(&[msg(7, "asker", "seeking: x", 5)], &[]).remove(0);
        assert_eq!(a.summary(100), "");
        a.waiting_on_asker = Some(Clarification {
            by: "h".into(),
            question: "q".into(),
        });
        a.room = Some(AskRoom {
            name: "ask-7".into(),
            members: 3,
            last_activity: 90,
        });
        let s = a.summary(100);
        assert!(
            s.contains("needs info") && s.contains("room ask-7 3 members"),
            "{s}"
        );
        assert!(
            a.room_active(100) && a.room_active(210),
            "news within two minutes"
        );
        assert!(!a.room_active(211), "a quiet room is not activity");
    }
    #[test]
    fn a_deadline_in_the_ask_text_is_read_as_utc() {
        // 2026-11-20 is 20777 days after the epoch
        let day = 20_777 * 86_400;
        let cases = [
            ("seeking: a review (by 2026-11-20)", Some(day + 86_399)),
            (
                "seeking: a review (by 2026-11-20 14:30)",
                Some(day + 14 * 3600 + 30 * 60),
            ),
            (
                "seeking: a (by 2026-11-19) b (by 2026-11-20)",
                Some(day + 86_399),
            ),
            ("seeking: a review by 2026-11-20", None),
            ("seeking: a review (by tomorrow)", None),
            ("seeking: a review (by 2026-13-40)", None),
            ("seeking: a review (by 2026-11-20 25:00)", None),
            ("seeking: nothing", None),
        ];
        for (text, want) in cases {
            assert_eq!(ask_deadline(text), want, "{text}");
        }
    }

    #[test]
    fn a_deadline_moves_the_stale_threshold_and_shows_on_the_summary() {
        let day = 20_777 * 86_400;
        let a = open_asks(
            &[msg(
                3,
                "asker",
                "seeking: x (by 2026-11-20)",
                day - 3 * 86_400,
            )],
            &[],
        )
        .remove(0);
        assert_eq!(
            a.stale_after(60),
            3 * 86_400 + 86_399,
            "the deadline replaces the room default"
        );
        assert!(
            a.summary(day - 86_400).contains("due in 1d"),
            "{}",
            a.summary(day - 86_400)
        );
        assert!(
            a.summary(day + 86_400 + 3600).contains("overdue 1h"),
            "{}",
            a.summary(day + 86_400 + 3600)
        );
        let plain = open_asks(&[msg(4, "asker", "seeking: y", 0)], &[]).remove(0);
        assert_eq!(plain.stale_after(60), 60);
        let past = open_asks(
            &[msg(
                5,
                "asker",
                "seeking: z (by 2026-11-20)",
                day + 86_400 * 2,
            )],
            &[],
        )
        .remove(0);
        assert_eq!(
            past.stale_after(60),
            60,
            "a deadline before the ask was posted never lowers the default"
        );
    }
}
