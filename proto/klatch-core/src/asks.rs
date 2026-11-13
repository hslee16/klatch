//! Asks: lobby messages that start with `seeking:`. klatch derives their state from the messages, the live
//! claims (DESIGN section 25.4). An ask is open until someone answers it or its asker closes it. Releasing an
//! `ask:<seq>` claim does not end it: a helper that gives up releases too.

use crate::{Claim, Message};

/// A message whose body starts with this text is an ask.
pub const ASK_PREFIX: &str = "seeking:";

/// How long an open ask may go without a claim before its asker is told.
pub const STALE_AFTER_SECS: i64 = 60;

/// How many distinct helpers must decline an ask as `cannot`, with none busy or failed, before klatch tells
/// the asker that nobody here may be able to do it.
pub const NO_CAPABLE_AFTER: usize = 2;

/// Why a helper did not take an ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeKind {
    /// `declined #N busy: ...`: capable, but not now. The ask stays open.
    Busy,
    /// `declined #N cannot: ...`: not capable, or not mine. The ask stays open for others.
    Cannot,
    /// `failed #N: ...`: tried and did not succeed. The ask stays open.
    Failed,
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

    /// How helpers answered the ask without taking it, for display: `[busy 1, cannot 2]`, or empty.
    pub fn outcome_summary(&self) -> String {
        let parts: Vec<String> = [
            ("busy", OutcomeKind::Busy),
            ("cannot", OutcomeKind::Cannot),
            ("failed", OutcomeKind::Failed),
        ]
        .iter()
        .filter_map(|(name, k)| {
            let n = self.distinct(*k);
            (n > 0).then(|| format!("{name} {n}"))
        })
        .collect();
        if parts.is_empty() {
            String::new()
        } else {
            format!("[{}]", parts.join(", "))
        }
    }

    /// Enough helpers said `cannot` and none said `busy` or `failed`: nobody here may be able to do this.
    pub fn no_capable_member(&self) -> bool {
        self.distinct(OutcomeKind::Cannot) >= NO_CAPABLE_AFTER
            && self.distinct(OutcomeKind::Busy) == 0
            && self.distinct(OutcomeKind::Failed) == 0
    }
}

/// A decline or failure message: its kind, the ask number, and the reason.
pub fn parse_outcome(body: &str) -> Option<(OutcomeKind, i64, String)> {
    let reason = |rest: &str| rest.trim().to_string();
    if let Some(rest) = body.strip_prefix("declined #") {
        let (n, after) = number(rest)?;
        let after = after.strip_prefix(' ')?;
        let (kind, rest) = if let Some(r) = after.strip_prefix("busy:") {
            (OutcomeKind::Busy, r)
        } else if let Some(r) = after.strip_prefix("cannot:") {
            (OutcomeKind::Cannot, r)
        } else {
            return None;
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
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Attribution;

    fn msg(seq: i64, sender: &str, body: &str, ts: i64) -> Message {
        Message {
            id: format!("id{seq}"),
            seq,
            sender: sender.into(),
            attribution: Attribution::Unattributed,
            label: "public".into(),
            body: body.into(),
            ts,
        }
    }

    fn claim(path: &str) -> Claim {
        Claim {
            path: path.into(),
            owner: "helper".into(),
            reason: String::new(),
            expires: 1000,
        }
    }

    #[test]
    fn an_ask_starts_with_the_prefix() {
        assert!(is_ask("seeking: a thing"));
        assert!(is_ask("seeking:no space"));
        assert!(!is_ask("not seeking: a thing"));
        assert!(!is_ask("Seeking: a thing"));
        assert!(!is_ask(" seeking: leading space"));
        assert!(!is_ask(""));
    }

    #[test]
    fn answers_and_closes_name_their_ask_by_number() {
        assert_eq!(answer_target("answer to #3: here"), Some(3));
        assert_eq!(answer_target("answer to #12:x"), Some(12));
        assert_eq!(answer_target("answer to #3 here"), None);
        assert_eq!(answer_target("answer to #: x"), None);
        assert_eq!(answer_target("my answer to #3: x"), None);
        assert_eq!(closed_target("closed #3"), Some(3));
        assert_eq!(closed_target("closed #3 thanks"), Some(3));
        assert_eq!(closed_target("closed #3."), Some(3));
        assert_eq!(closed_target("closed #x"), None);
        assert_eq!(closed_target("not closed #3"), None);
        assert_eq!(closed_target("closed #"), None);
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
        assert_eq!(
            parse_outcome("declined #4 busy: on LIB-2"),
            Some((OutcomeKind::Busy, 4, "on LIB-2".into()))
        );
        assert_eq!(
            parse_outcome("declined #12 cannot: not my area"),
            Some((OutcomeKind::Cannot, 12, "not my area".into()))
        );
        assert_eq!(
            parse_outcome("failed #3: the build broke"),
            Some((OutcomeKind::Failed, 3, "the build broke".into()))
        );
        assert_eq!(parse_outcome("declined #4: no class"), None);
        assert_eq!(parse_outcome("declined #x busy: r"), None);
        assert_eq!(parse_outcome("failed #3 the build broke"), None);
        assert_eq!(parse_outcome("I declined #4 busy: r"), None);
        assert_eq!(
            parse_outcome("declined #4 busy:"),
            Some((OutcomeKind::Busy, 4, String::new()))
        );
    }

    fn with(kind: OutcomeKind, ask: i64, who: &str) -> Message {
        let word = match kind {
            OutcomeKind::Busy => format!("declined #{ask} busy: r"),
            OutcomeKind::Cannot => format!("declined #{ask} cannot: r"),
            OutcomeKind::Failed => format!("failed #{ask}: r"),
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
}
