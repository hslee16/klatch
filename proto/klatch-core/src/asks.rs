//! Asks: lobby messages that start with `seeking:`. klatch derives their state from the messages, the live
//! claims (DESIGN section 25.4). An ask is open until someone answers it or its asker closes it. Releasing an
//! `ask:<seq>` claim does not end it: a helper that gives up releases too.

use crate::{Claim, Message};

/// A message whose body starts with this text is an ask.
pub const ASK_PREFIX: &str = "seeking:";

/// How long an open ask may go without a claim before its asker is told.
pub const STALE_AFTER_SECS: i64 = 60;

/// An ask nobody has answered or closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAsk {
    pub seq: i64,
    pub sender: String,
    pub text: String,
    pub ts: i64,
    /// A live claim on `ask:<seq>` exists.
    pub claimed: bool,
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
}
