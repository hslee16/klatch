//! The one list of convention words klatch understands (#238). CONVENTIONS.md describes each of them and a test
//! (`tests/conventions_doc.rs`) fails when the document and this list disagree, so a prompt, the document and the
//! code cannot drift apart. A word here is one klatch reads and derives something from; a word agents use that
//! klatch does not read is not a convention word (it is only text).
//!
//! Adding a word: add a row here and in CONVENTIONS.md, with an example that `klatch_reads` accepts. Old agents
//! never post it, so nothing they do changes; a new word must never change what an existing word means.

use crate::domain::{
    ASK_LEASE_DEFAULT_SECS, Message, OutcomeKind, answer_target, claim_ttl, closed_target,
    current_offers, is_ask, open_prs, open_tasks, parse_clarification, parse_outcome, parse_parked,
    reopen_target, statuses,
};

/// One word: how it is written, an example, and the question "does the code read this example as this word".
pub struct ConventionWord {
    /// The word as an agent writes it, with `N` for a number; the first column of CONVENTIONS.md.
    pub form: &'static str,
    /// A complete message (or claim path) that uses the word.
    pub example: &'static str,
    reads: fn(&str) -> bool,
}

impl ConventionWord {
    /// Whether the code treats `example` as this word.
    pub fn klatch_reads(&self) -> bool {
        (self.reads)(self.example)
    }
}

fn msg(seq: i64, sender: &str, body: &str) -> Message {
    Message {
        id: format!("id{seq}"),
        seq,
        sender: sender.into(),
        attribution: crate::domain::Attribution::Unattributed,
        label: "public".into(),
        body: body.into(),
        ts: 1000 + seq,
        kind: None,
        payload: None,
    }
}

/// Whether a word posted after `working: T` changes the tasks klatch derives.
fn changes_tasks(body: &str) -> bool {
    let start = msg(1, "a", "working: T");
    let before = format!("{:?}", open_tasks(std::slice::from_ref(&start)));
    let after = format!("{:?}", open_tasks(&[start, msg(2, "a", body)]));
    before != after
}

fn outcome(kind: OutcomeKind) -> impl Fn(&str) -> bool {
    move |b| parse_outcome(b).is_some_and(|(k, n, _)| k == kind && n == 3)
}

fn offered(body: &str) -> bool {
    !current_offers(&[msg(1, "a", body)], 1001, 100).is_empty()
}

fn withdrawn(body: &str) -> bool {
    let offers = [msg(1, "a", "offering: x"), msg(2, "a", body)];
    current_offers(&offers, 1002, 100).is_empty()
}

fn status_done(body: &str) -> bool {
    statuses(&[msg(1, "a", body)])
        .iter()
        .any(|s| s.status == "done")
}

fn pr_opened(body: &str) -> bool {
    open_prs(&[msg(1, "a", body)]).iter().any(|p| p.number == 7)
}

fn pr_merged(body: &str) -> bool {
    open_prs(&[msg(1, "a", "pr: #7 t"), msg(2, "a", body)]).is_empty()
}

macro_rules! word {
    ($form:expr, $example:expr, $reads:expr) => {
        ConventionWord {
            form: $form,
            example: $example,
            reads: $reads,
        }
    };
}

/// Every convention word, in the order CONVENTIONS.md lists them.
pub static CONVENTION_WORDS: &[ConventionWord] = &[
    word!("seeking:", "seeking: a slugify helper", is_ask),
    word!("answer to #N:", "answer to #3: use the slug crate", |b| {
        answer_target(b) == Some(3)
    }),
    word!("closed #N", "closed #3", |b| closed_target(b) == Some(3)),
    word!("reopen #N:", "reopen #3: that is the wrong crate", |b| {
        reopen_target(b) == Some(3)
    }),
    word!("need-info #N:", "need-info #3: which locale?", |b| {
        parse_clarification(b).is_some_and(|(need, n, _)| need && n == 3)
    }),
    word!("info #N:", "info #3: en-US", |b| {
        parse_clarification(b).is_some_and(|(need, n, _)| !need && n == 3)
    }),
    word!("partial #N:", "partial #3: parser done", |b| {
        outcome(OutcomeKind::Partial)(b)
    }),
    word!("declined #N busy:", "declined #3 busy: in a build", |b| {
        outcome(OutcomeKind::Busy)(b)
    }),
    word!(
        "declined #N cannot:",
        "declined #3 cannot: not my area",
        |b| { outcome(OutcomeKind::Cannot)(b) }
    ),
    word!("failed #N:", "failed #3: the test still fails", |b| {
        outcome(OutcomeKind::Failed)(b)
    }),
    word!("offering:", "offering: text helpers", offered),
    word!("withdrawn:", "withdrawn: busy now", withdrawn),
    word!("status: done", "status: done", status_done),
    word!("parked:", "parked: nightly run, back by 2h", |b| {
        parse_parked(b, 1000).is_some()
    }),
    word!("working:", "working: write the docs", changes_tasks),
    word!("checkpoint:", "checkpoint: parser done", changes_tasks),
    word!("handoff:", "handoff: T to bob", changes_tasks),
    word!("finished:", "finished: T", changes_tasks),
    word!("pr: #N", "pr: #7 add the thing", pr_opened),
    word!("merged: #N", "merged: #7", pr_merged),
    word!("ask:N", "ask:3", |p| claim_ttl(p, None)
        == ASK_LEASE_DEFAULT_SECS),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_code_reads_every_example_as_its_word() {
        for w in CONVENTION_WORDS {
            assert!(w.klatch_reads(), "{}: `{}` is not read", w.form, w.example);
        }
    }

    #[test]
    fn forms_are_unique() {
        let mut forms: Vec<_> = CONVENTION_WORDS.iter().map(|w| w.form).collect();
        forms.sort_unstable();
        forms.dedup();
        assert_eq!(forms.len(), CONVENTION_WORDS.len());
    }

    #[test]
    fn a_plain_message_is_no_word() {
        for w in CONVENTION_WORDS {
            let plain = ConventionWord {
                form: w.form,
                example: "hello there",
                reads: w.reads,
            };
            assert!(!plain.klatch_reads(), "{} reads plain text", w.form);
        }
    }
}
