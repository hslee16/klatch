//! A pull request's lifecycle on the board (DESIGN section 32 item 84, #707). Two log words and one claim:
//! `pr: #N <title>` says the sender opened PR N, a claim on the path `pr:N` is the lease the owner keeps while the
//! PR is open, and `merged: #N` (or releasing the claim with a note that starts with `merged`) says it is over. An
//! open PR with no live `pr:N` claim is an obligation nobody holds. Nothing is stored and nothing is read from the
//! hosting service: the member says what happened, the log keeps it, and the claim table says who holds it now.

use crate::domain::hook::clip;
use crate::domain::parked::span;
use crate::domain::{Message, flat};

/// A message starting with this text says its sender opened a pull request.
pub const PR_PREFIX: &str = "pr:";
/// A message starting with this text says a pull request is over (merged, or closed without merging).
pub const MERGED_PREFIX: &str = "merged:";

/// The prefixes of every word that `open_prs` folds, for a store query that reads only those messages.
pub const PR_WORDS: [&str; 2] = [PR_PREFIX, MERGED_PREFIX];

/// A pull request that nobody said was over, with who holds its claim now (set by the reader).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenPr {
    pub number: u64,
    /// The member that said it opened the PR.
    pub opener: String,
    pub title: String,
    /// When the `pr:` word was posted.
    pub ts: i64,
    /// The member holding a live claim on `pr:N`, if any.
    pub holder: Option<String>,
}

impl OpenPr {
    /// A PR said to be open by `opener` at `ts`, held by nobody yet.
    pub fn new(number: u64, opener: &str, title: &str, ts: i64) -> Self {
        Self {
            number,
            opener: opener.into(),
            title: title.into(),
            ts,
            holder: None,
        }
    }

    /// The claim path that holds this PR.
    pub fn path(&self) -> String {
        format!("pr:{}", self.number)
    }

    /// One line for the board: who holds it, or that it has no owner.
    pub fn line(&self, now: i64) -> String {
        let title = clip(&self.title);
        let state = match &self.holder {
            Some(h) => format!("held by {}", flat(h)),
            None => format!(
                "open without an owner for {}, opened by {}",
                span(now - self.ts),
                flat(&self.opener)
            ),
        };
        format!("#{} ({state}): {title}", self.number)
    }

    /// The line a digest shows for an unowned PR: what it is and what to do about it.
    pub fn obligation(&self, now: i64) -> String {
        format!(
            "open PR without an owner: #{} \"{}\" (opened by {} {} ago); take it with `claim pr:{} --shared`, or release it with --note merged if it is over",
            self.number,
            clip(&self.title),
            flat(&self.opener),
            span(now - self.ts),
            self.number
        )
    }
}

/// The PR number a word names: `N`, `#N` or `PR N`, then the end of the text or a space; and the text after it.
fn number_of(text: &str) -> Option<(u64, &str)> {
    let t = text.trim_start();
    let t = t.strip_prefix("PR ").unwrap_or(t).trim_start();
    let t = t.strip_prefix('#').unwrap_or(t);
    let digits: String = t.chars().take_while(char::is_ascii_digit).collect();
    let rest = &t[digits.len()..];
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some((digits.parse().ok()?, rest.trim()))
}

/// Whether a release note says the PR is over.
pub fn is_merged_note(note: &str) -> bool {
    note.trim_start().to_lowercase().starts_with("merged")
}

/// The pull requests in `messages` that were opened and not ended, oldest first. The holder is left unset.
pub fn open_prs(messages: &[Message]) -> Vec<OpenPr> {
    let mut ordered: Vec<&Message> = messages.iter().collect();
    ordered.sort_by_key(|m| m.seq);
    let mut prs: Vec<OpenPr> = Vec::new();
    for m in ordered {
        if let Some((number, title)) = m.body.strip_prefix(PR_PREFIX).and_then(number_of) {
            if !prs.iter().any(|p| p.number == number) {
                prs.push(OpenPr::new(number, &m.sender, title, m.ts));
            }
        } else if let Some((number, _)) = m.body.strip_prefix(MERGED_PREFIX).and_then(number_of) {
            prs.retain(|p| p.number != number);
        }
    }
    prs
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

    fn log(entries: &[(&str, &str)]) -> Vec<Message> {
        entries
            .iter()
            .enumerate()
            .map(|(i, (who, body))| msg(i as i64 + 1, who, body, 100 + i as i64 * 10))
            .collect()
    }

    #[test]
    fn a_pr_word_opens_a_pr_with_its_opener_title_and_time() {
        let prs = open_prs(&log(&[("alice", "pr: #707 claim the pr lifecycle")]));
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].number, 707);
        assert_eq!(prs[0].opener, "alice");
        assert_eq!(prs[0].title, "claim the pr lifecycle");
        assert_eq!(prs[0].ts, 100);
        assert_eq!(prs[0].holder, None);
        assert_eq!(prs[0].path(), "pr:707");
    }

    #[test]
    fn the_number_may_be_written_with_or_without_the_hash_or_as_pr_n() {
        for body in ["pr: 12 t", "pr: #12 t", "pr: PR 12 t", "pr: PR #12 t"] {
            let prs = open_prs(&log(&[("a", body)]));
            assert_eq!(prs.len(), 1, "{body}");
            assert_eq!(prs[0].number, 12, "{body}");
            assert_eq!(prs[0].title, "t", "{body}");
        }
    }

    #[test]
    fn text_that_is_not_a_pr_number_opens_nothing() {
        for body in [
            "pr: soon",
            "pr: 12abc",
            "pr:",
            "pr: #",
            "pull: #12",
            "PR: 12",
        ] {
            assert!(open_prs(&log(&[("a", body)])).is_empty(), "{body}");
        }
    }

    #[test]
    fn a_merged_word_from_anyone_ends_the_pr() {
        let prs = open_prs(&log(&[
            ("alice", "pr: #7 one"),
            ("alice", "pr: #8 two"),
            ("bob", "merged: #7"),
        ]));
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].number, 8);
    }

    #[test]
    fn a_pr_opened_again_after_it_ended_is_open_again_and_a_repeat_keeps_the_first_opener() {
        let prs = open_prs(&log(&[
            ("alice", "pr: #7 one"),
            ("bob", "pr: #7 one again"),
            ("bob", "merged: #7"),
            ("carol", "pr: #7 reopened"),
        ]));
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].opener, "carol");
        let prs = open_prs(&log(&[
            ("alice", "pr: #7 one"),
            ("bob", "pr: #7 one again"),
        ]));
        assert_eq!(prs[0].opener, "alice");
        assert_eq!(prs[0].ts, 100);
    }

    #[test]
    fn a_release_note_that_starts_with_merged_says_the_pr_is_over() {
        assert!(is_merged_note("merged"));
        assert!(is_merged_note("  Merged into main"));
        assert!(!is_merged_note("handing off, not merged"));
        assert!(!is_merged_note(""));
    }

    #[test]
    fn the_board_line_says_who_holds_the_pr_or_that_it_has_no_owner() {
        let mut pr = open_prs(&log(&[("alice", "pr: #7 one")])).remove(0);
        assert_eq!(
            pr.line(100 + 7200),
            "#7 (open without an owner for 2h, opened by alice): one"
        );
        pr.holder = Some("bob".into());
        assert_eq!(pr.line(500), "#7 (held by bob): one");
        let o = pr.obligation(100 + 7200);
        assert!(o.contains("open PR without an owner: #7"), "{o}");
        assert!(o.contains("claim pr:7 --shared"), "{o}");
    }
}
