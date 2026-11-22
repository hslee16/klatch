//! Finding earlier answered asks (#239): `klatch find <room> <text>` lists asks in the room's history that someone
//! already answered and that look like `text`, so an asker can use the answer instead of asking again. Pure and
//! read only; it posts nothing and never decides for the asker.
//!
//! The similarity rule is deliberately plain and does not guess. An ask matches when either
//! 1. the query, lower-cased with its whitespace folded, is a substring of the ask text (same folding), or
//! 2. the ask and the query share at least `min(2, keywords in the query)` keywords, where a keyword is a
//!    run of letters and digits at least `KEYWORD_MIN_LEN` characters long, compared lower-cased.
//!
//! No stemming, no synonyms, no ranking by meaning: a match is shown with the reason (`substring` or the shared
//! words) so the asker judges it. An ask nobody answered, or one reopened after its answer, is not listed.

use crate::domain::{
    ASK_PREFIX, CloseOutcome, Message, answer_target, is_ask, latest_close, reopen_target,
};
use std::collections::BTreeSet;

/// Shortest word that counts as a keyword.
pub const KEYWORD_MIN_LEN: usize = 4;

/// An earlier ask that was answered, with its latest answer and why it matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnsweredAsk {
    pub seq: i64,
    pub sender: String,
    pub text: String,
    pub answer_seq: i64,
    pub answer_by: String,
    pub answer: String,
    /// How the asker closed the ask after that answer (#232), if they did.
    pub closed: Option<CloseOutcome>,
    /// `substring`, or the shared keywords joined by commas.
    pub why: String,
}

/// The answered asks in `messages` that match `query`, newest ask first. The ask text shown and matched is the
/// body after the `seeking:` prefix.
pub fn find_answered(messages: &[Message], query: &str) -> Vec<AnsweredAsk> {
    let mut out = Vec::new();
    for ask in messages.iter().filter(|m| is_ask(&m.body)) {
        let latest_reopen = messages
            .iter()
            .filter(|m| m.sender == ask.sender && reopen_target(&m.body) == Some(ask.seq))
            .map(|m| m.seq)
            .max();
        let answer = messages
            .iter()
            .filter(|m| m.sender != ask.sender && answer_target(&m.body) == Some(ask.seq))
            .filter(|m| latest_reopen.is_none_or(|r| m.seq > r))
            .max_by_key(|m| m.seq);
        let Some(answer) = answer else { continue };
        let text = ask.body[ASK_PREFIX.len()..].trim();
        let Some(why) = why_matches(query, text) else {
            continue;
        };
        let body = answer.body.as_str();
        let after = body.find(':').map_or(body, |i| &body[i + 1..]).trim();
        out.push(AnsweredAsk {
            seq: ask.seq,
            sender: ask.sender.clone(),
            text: text.to_string(),
            answer_seq: answer.seq,
            answer_by: answer.sender.clone(),
            answer: after.to_string(),
            closed: latest_close(messages, ask),
            why,
        });
    }
    out.sort_by_key(|a| std::cmp::Reverse(a.seq));
    out
}

fn fold(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn keywords(s: &str) -> BTreeSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= KEYWORD_MIN_LEN)
        .map(str::to_string)
        .collect()
}

/// Why `ask_text` matches `query`, or `None` when it does not.
fn why_matches(query: &str, ask_text: &str) -> Option<String> {
    let q = fold(query);
    if q.is_empty() {
        return None;
    }
    if fold(ask_text).contains(&q) {
        return Some("substring".into());
    }
    let wanted = keywords(query);
    if wanted.is_empty() {
        return None;
    }
    let shared: Vec<String> = wanted.intersection(&keywords(ask_text)).cloned().collect();
    (shared.len() >= wanted.len().min(2)).then(|| shared.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Attribution;

    fn msg(seq: i64, sender: &str, body: &str) -> Message {
        Message {
            id: format!("id{seq}"),
            seq,
            sender: sender.into(),
            attribution: Attribution::Unattributed,
            label: "public".into(),
            body: body.into(),
            ts: 1000 + seq,
            kind: None,
            payload: None,
            advisory: false,
            scope: None,
        }
    }

    fn room() -> Vec<Message> {
        vec![
            msg(
                1,
                "alice",
                "seeking: which port does the Gateway listen on?",
            ),
            msg(2, "bob", "answer to #1: 7878"),
            msg(3, "carol", "seeking: who owns the release pipeline"),
            msg(4, "alice", "seeking: how do I rotate the signing key"),
            msg(5, "dave", "answer to #4: klatch admin rotate-key"),
        ]
    }

    #[test]
    fn a_substring_of_an_answered_ask_matches_ignoring_case_and_spacing() {
        let found = find_answered(&room(), "  GATEWAY   listen ");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].seq, 1);
        assert_eq!(found[0].why, "substring");
        assert_eq!(found[0].answer, "7878");
        assert_eq!(found[0].answer_by, "bob");
        assert_eq!(found[0].answer_seq, 2);
    }

    #[test]
    fn two_shared_keywords_match_without_a_substring() {
        let found = find_answered(&room(), "rotate signing keys please");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].seq, 4);
        assert_eq!(found[0].why, "rotate,signing");
    }

    #[test]
    fn one_shared_keyword_is_not_enough_when_the_query_has_two() {
        assert!(find_answered(&room(), "rotate the wheels").is_empty());
    }

    #[test]
    fn a_one_keyword_query_needs_that_keyword() {
        assert_eq!(find_answered(&room(), "gateway").len(), 1);
        assert!(find_answered(&room(), "kubernetes").is_empty());
    }

    #[test]
    fn short_words_never_count_as_keywords() {
        assert!(find_answered(&room(), "do it on a").is_empty());
    }

    #[test]
    fn an_empty_query_matches_nothing() {
        assert!(find_answered(&room(), "   ").is_empty());
    }

    #[test]
    fn an_unanswered_ask_is_not_listed() {
        assert!(find_answered(&room(), "release pipeline").is_empty());
    }

    #[test]
    fn the_askers_own_answer_or_a_close_does_not_count_as_answered() {
        let mut m = room();
        m.push(msg(6, "carol", "answer to #3: me again"));
        m.push(msg(7, "carol", "closed #3"));
        assert!(find_answered(&m, "release pipeline").is_empty());
    }

    #[test]
    fn an_answered_ask_shows_how_the_asker_closed_it() {
        let mut m = room();
        assert_eq!(find_answered(&m, "gateway listen")[0].closed, None);
        m.push(msg(6, "alice", "closed #1 ok"));
        assert_eq!(
            find_answered(&m, "gateway listen")[0].closed,
            Some(CloseOutcome::Ok)
        );
        m.push(msg(7, "alice", "reopen #1: wrong"));
        m.push(msg(8, "erin", "answer to #1: 9000"));
        m.push(msg(9, "alice", "closed #1 unresolved"));
        assert_eq!(
            find_answered(&m, "gateway listen")[0].closed,
            Some(CloseOutcome::Unresolved)
        );
    }

    #[test]
    fn a_reopened_ask_is_not_listed_until_answered_again() {
        let mut m = room();
        m.push(msg(6, "alice", "reopen #1: that port is wrong"));
        assert!(find_answered(&m, "gateway listen").is_empty());
        m.push(msg(7, "erin", "answer to #1: 9000"));
        let found = find_answered(&m, "gateway listen");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].answer, "9000");
    }

    #[test]
    fn the_latest_answer_is_shown_and_newest_ask_first() {
        let mut m = room();
        m.push(msg(6, "erin", "answer to #1: 9000"));
        m.push(msg(7, "frank", "seeking: which port does the gateway use"));
        m.push(msg(8, "bob", "answer to #7: 7878"));
        let found = find_answered(&m, "which port gateway");
        assert_eq!(found.iter().map(|a| a.seq).collect::<Vec<_>>(), vec![7, 1]);
        assert_eq!(found[1].answer, "9000");
    }
}
