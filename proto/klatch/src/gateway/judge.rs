//! Draining the review queue (#48, R31): each pending item is shown to a `Judge` once and, unless the judge defers,
//! gets its verdict recorded and audited. A daemon is just a loop around `drain`. Nothing here enforces labels: that
//! stays inline in the gateway and fails closed, so an absent or unreachable judge only leaves the queue longer.

use crate::domain::{Judge, LabelChain, Result, ReviewItem, RulesJudge, Verdict};
use crate::store::{NewAudit, QueuedReview, ReviewResolution, Store};

/// How many items one drain looks at, so a run is bounded.
pub const DRAIN_LIMIT: usize = 100;

/// What one drain did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Drained {
    pub allowed: usize,
    pub redacted: usize,
    pub denied: usize,
    /// Left in the queue because the judge made no decision.
    pub deferred: usize,
}

impl Drained {
    pub fn decided(&self) -> usize {
        self.allowed + self.redacted + self.denied
    }
}

/// The item as the judge sees it, or `None` when the message is no longer there (archived rows keep a tombstone, a
/// deleted room keeps nothing).
pub(crate) fn load_item<S: Store>(store: &S, q: &QueuedReview) -> Result<Option<ReviewItem>> {
    let found = store.tail(&q.room, q.seq - 1, 1)?;
    // a room's canary is no message anyone may point at (#954)
    Ok(found
        .into_iter()
        .find(|m| m.seq == q.seq && m.kind.as_deref() != Some(crate::gateway::CANARY_KIND))
        .map(|m| ReviewItem {
            id: q.id,
            room: q.room.clone(),
            seq: q.seq,
            to_label: q.to_label.clone(),
            from_label: m.label,
            reason: q.reason.clone(),
            text: m.body,
        }))
}

/// Run every pending item past `judge` once. A vanished message is denied: there is nothing left to let across.
pub fn drain<S: Store>(store: &S, judge: &dyn Judge, now: i64) -> Result<Drained> {
    let mut out = Drained::default();
    for q in store.pending_reviews(DRAIN_LIMIT)? {
        let (verdict, detail, note) = match load_item(store, &q)? {
            Some(item) => {
                let (verdict, note) = judge.judge_noted(&item);
                (verdict, String::new(), note)
            }
            None => (
                Verdict::Deny("message missing".into()),
                "message missing".to_string(),
                None,
            ),
        };
        if let (Verdict::Defer, Some(note)) = (&verdict, &note) {
            audit_deferral(store, &q, note, now)?;
        }
        match &verdict {
            Verdict::Defer => out.deferred += 1,
            Verdict::Allow => out.allowed += 1,
            Verdict::Redact(_) => out.redacted += 1,
            Verdict::Deny(_) => out.denied += 1,
        }
        if verdict != Verdict::Defer {
            record(store, &q, (&verdict, &detail), ("judge", now))?;
        }
    }
    Ok(out)
}

/// Audit a deliberate deferral (a hosted verdict below its confidence band, #1013) once per queued item: the item
/// stays queued for the reviewer and a resident loop sees it again every pass, so a row already written for it
/// since it was queued is not repeated. The note holds no message text.
fn audit_deferral<S: Store>(store: &S, q: &QueuedReview, note: &str, now: i64) -> Result<()> {
    let target = format!("{}#{}", q.room, q.seq);
    if store.last_audit_ts("judge-defer", &target, q.ts)?.is_some() {
        return Ok(());
    }
    let detail = format!("defer to {}: {note}", q.to_label);
    store.record_audit(&NewAudit::new("judge-defer", &target, &detail), now)
}

/// The local rules judge as the store configures it: the label chain and the scrub list, read once for a pass.
pub fn rules_judge<S: Store>(store: &S) -> Result<RulesJudge> {
    let chain = match store.policy_get("chain")? {
        Some(text) => LabelChain::parse(&text).map_err(crate::domain::Error::Backend)?,
        None => LabelChain::default(),
    };
    Ok(RulesJudge::new(store.policy_list("scrub:")?, chain))
}

/// Keep the verdict word and a reason; never the text, which the audit must not hold. `action` says who ruled.
pub(crate) fn record<S: Store>(
    store: &S,
    q: &QueuedReview,
    (v, detail): (&Verdict, &str),
    (action, now): (&str, i64),
) -> Result<()> {
    let reason = match v {
        Verdict::Deny(why) if detail.is_empty() => why.as_str(),
        _ => detail,
    };
    if let Verdict::Redact(text) = v {
        // kept on the item so a crossing can release the redaction; the audit never holds it
        store.set_review_text(q.id, text)?;
    }
    store.resolve_review(&ReviewResolution::new(q.id, v.word(), reason), now)?;
    let target = format!("{}#{}", q.room, q.seq);
    let note = format!("{} to {}: {}", v.word(), q.to_label, reason);
    store.record_audit(
        &NewAudit::new(action, &target, note.trim_end_matches(": ")),
        now,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Judge, NullJudge, ReviewItem, Verdict};
    use crate::store::areas::*;
    use crate::store::{NewReview, SqliteStore};

    struct Says(Verdict);
    impl Judge for Says {
        fn judge(&self, _: &ReviewItem) -> Verdict {
            self.0.clone()
        }
    }

    /// Repeats the text it was shown, so a test can see what the judge got.
    struct Echo;
    impl Judge for Echo {
        fn judge(&self, item: &ReviewItem) -> Verdict {
            Verdict::Deny(item.text.clone())
        }
    }

    fn queued() -> SqliteStore {
        let s = SqliteStore::open_in_memory().unwrap();
        s.join("r", "alice", 1).unwrap();
        s.post(crate::store::Seat::new("r", "alice"), "hello", 2)
            .unwrap();
        s.enqueue_review(&NewReview::new("r", 1, "public", "crosses"), 3)
            .unwrap();
        s
    }

    #[test]
    fn the_null_judge_leaves_every_item_queued_and_nothing_else_changes() {
        let s = queued();
        let out = drain(&s, &NullJudge, 10).unwrap();
        assert_eq!((out.decided(), out.deferred), (0, 1));
        assert_eq!(s.pending_reviews(10).unwrap().len(), 1);
        assert_eq!(s.tail("r", 0, 10).unwrap().len(), 1);
    }

    #[test]
    fn a_verdict_resolves_the_item_and_is_audited() {
        let s = queued();
        let out = drain(&s, &Says(Verdict::Deny("scrubber".into())), 10).unwrap();
        assert_eq!((out.denied, out.deferred), (1, 0));
        assert!(s.pending_reviews(10).unwrap().is_empty());
        let log = s.audit_log(5).unwrap();
        assert_eq!(log[0].action, "judge");
        assert!(log[0].detail.contains("deny"), "{}", log[0].detail);
        assert!(
            !log[0].detail.contains("hello"),
            "the audit holds message text"
        );
    }

    /// Defers on purpose with a note, as the hosted judge does below its confidence band (#1013).
    struct Unsure;
    impl Judge for Unsure {
        fn judge(&self, _: &ReviewItem) -> Verdict {
            Verdict::Defer
        }
        fn judge_noted(&self, _: &ReviewItem) -> (Verdict, Option<String>) {
            (
                Verdict::Defer,
                Some("hosted deny at confidence 0.40, below 0.70".into()),
            )
        }
    }

    #[test]
    fn a_deliberate_deferral_stays_queued_and_is_audited_once() {
        let s = queued();
        for now in [10, 20] {
            let out = drain(&s, &Unsure, now).unwrap();
            assert_eq!((out.decided(), out.deferred), (0, 1));
        }
        assert_eq!(s.pending_reviews(10).unwrap().len(), 1);
        let log = s.audit_log(10).unwrap();
        let notes: Vec<_> = log.iter().filter(|l| l.action == "judge-defer").collect();
        assert_eq!(notes.len(), 1, "{log:?}");
        assert_eq!(notes[0].target, "r#1");
        assert!(notes[0].detail.contains("defer") && notes[0].detail.contains("0.40"));
        assert!(
            !notes[0].detail.contains("hello"),
            "the audit holds message text"
        );
    }

    #[test]
    fn a_plain_deferral_writes_no_audit_row() {
        let s = queued();
        drain(&s, &NullJudge, 10).unwrap();
        assert!(
            s.audit_log(10)
                .unwrap()
                .iter()
                .all(|l| l.action != "judge-defer")
        );
    }

    #[test]
    fn the_judge_is_shown_the_message_text_and_a_vanished_message_is_denied() {
        let s = queued();
        s.enqueue_review(&NewReview::new("r", 99, "public", "gone"), 4)
            .unwrap();
        let out = drain(&s, &Echo, 10).unwrap();
        assert_eq!(out.denied, 2);
        let log = s.audit_log(5).unwrap();
        assert!(log.iter().any(|l| l.detail.contains("message missing")));
    }

    #[test]
    fn the_store_rules_judge_denies_a_scrub_match_and_defers_clean_text_and_a_failing_judge_allows_nothing()
     {
        let s = queued();
        s.policy_set("scrub:acme", Some("hello"), 1).unwrap();
        let rules = rules_judge(&s).unwrap();
        let out = drain(&s, &rules, 10).unwrap();
        assert_eq!((out.denied, out.allowed), (1, 0));
        let s = queued();
        let out = drain(&s, &rules_judge(&s).unwrap(), 10).unwrap();
        assert_eq!((out.decided(), out.deferred), (0, 1));
        let flaky = crate::domain::Breaker::new(&NullJudge, 1);
        let out = drain(&s, &flaky, 11).unwrap();
        assert_eq!((out.decided(), out.deferred), (0, 1));
        assert_eq!(s.pending_reviews(10).unwrap().len(), 1);
    }

    #[test]
    fn allow_and_redact_are_counted_apart() {
        let s = queued();
        assert_eq!(drain(&s, &Says(Verdict::Allow), 10).unwrap().allowed, 1);
        s.enqueue_review(&NewReview::new("r", 1, "public", "again"), 11)
            .unwrap();
        let out = drain(&s, &Says(Verdict::Redact("[x]".into())), 12).unwrap();
        assert_eq!((out.redacted, out.allowed), (1, 0));
    }
}
