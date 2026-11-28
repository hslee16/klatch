//! Draining the review queue (#48, R31): each pending item is shown to a `Judge` once and, unless the judge defers,
//! gets its verdict recorded and audited. A daemon is just a loop around `drain`. Nothing here enforces labels: that
//! stays inline in the gateway and fails closed, so an absent or unreachable judge only leaves the queue longer.

use crate::domain::{Judge, LabelChain, Result, ReviewItem, RulesJudge, Verdict};
use crate::store::{NewAudit, QueuedReview, ReviewResolution, ReviewStore, Store};

/// How many items one page of the queue holds. A drain walks every page by age (#1736), so items behind a full page
/// of deferred ones are still judged; the run is bounded by the per-requester cap on waiting crossings.
pub const DRAIN_LIMIT: usize = 100;

/// The most pages one walk of the queue takes: far more than the cap allows, so reaching it means a broken cursor.
const MAX_PAGES: usize = 100_000;

/// A walk over the waiting queue by age, a page of `DRAIN_LIMIT` at a time (#1736). A store that returns an item at or
/// before the cursor, or a walk that never ends, is an error and not a hang: failing closed leaves the queue as it is.
pub(crate) struct PendingPages {
    after: i64,
    taken: usize,
}

impl PendingPages {
    pub(crate) fn new() -> Self {
        Self { after: 0, taken: 0 }
    }

    /// The next page, or `None` at the end of the queue.
    pub(crate) fn next<S: ReviewStore>(&mut self, store: &S) -> Result<Option<Vec<QueuedReview>>> {
        let page = store.pending_reviews_after(self.after, DRAIN_LIMIT)?;
        let (Some(first), Some(last)) = (page.first(), page.last()) else {
            return Ok(None);
        };
        self.taken += 1;
        if first.id <= self.after || last.id < first.id || self.taken > MAX_PAGES {
            return Err(crate::domain::Error::Backend(
                "the review queue did not page forward".into(),
            ));
        }
        self.after = last.id;
        Ok(Some(page))
    }
}

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
            to_room: q.to_room.clone(),
        }))
}

/// Run every pending item past `judge` once. A vanished message is denied: there is nothing left to let across.
pub fn drain<S: Store>(store: &S, judge: &dyn Judge, now: i64) -> Result<Drained> {
    let mut out = Drained::default();
    let mut pages = PendingPages::new();
    while let Some(page) = pages.next(store)? {
        for q in page {
            drain_one(store, judge, &q, now, &mut out)?;
        }
    }
    Ok(out)
}

fn drain_one<S: Store>(
    store: &S,
    judge: &dyn Judge,
    q: &QueuedReview,
    now: i64,
    out: &mut Drained,
) -> Result<()> {
    let (verdict, detail, note) = match load_item(store, q)? {
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
    // A deferral with no note leaves an earlier note in place: it was true when written, the audit holds the
    // sequence, and clearing it would only cost a write on every pass.
    if let (Verdict::Defer, Some(note)) = (&verdict, &note) {
        audit_deferral(store, q, note, now)?;
        store.set_review_flag(q.id, &flag_text(note))?;
    }
    match &verdict {
        Verdict::Defer => out.deferred += 1,
        Verdict::Allow => out.allowed += 1,
        Verdict::Redact(_) => out.redacted += 1,
        Verdict::Deny(_) => out.denied += 1,
    }
    if verdict != Verdict::Defer {
        record(store, q, (&verdict, &detail), ("judge", now))?;
    }
    Ok(())
}

/// The most a reviewer is shown of a judge's deferral note.
const FLAG_MAX: usize = 120;

/// A deferral note as a reviewer reads it: plain letters, digits and `:_-;. ` only, capped. The note is derived from the
/// screen's rule name (or a hosted judge's own fixed words), never from message text, but a reviewer reads it as an
/// instruction-shaped line, so anything else is dropped.
pub(crate) fn flag_text(note: &str) -> String {
    note.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '-' | ';' | '.' | ' '))
        .take(FLAG_MAX)
        .collect::<String>()
        .trim()
        .to_string()
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

    /// A store whose queue ignores the cursor, as a broken `pending_reviews_after` would.
    struct IgnoresCursor;
    impl crate::store::ReviewStore for IgnoresCursor {
        fn pending_reviews_after(&self, _after: i64, _limit: usize) -> Result<Vec<QueuedReview>> {
            Ok(vec![QueuedReview {
                id: 1,
                ..QueuedReview::default()
            }])
        }
    }

    #[test]
    fn a_cursor_that_does_not_advance_is_an_error_not_a_hang() {
        let mut pages = PendingPages::new();
        assert!(pages.next(&IgnoresCursor).unwrap().is_some());
        assert!(pages.next(&IgnoresCursor).is_err());
    }

    /// Pages forward forever (a new id each call), as a queue that never ends.
    struct Endless(std::cell::Cell<i64>);
    impl crate::store::ReviewStore for Endless {
        fn pending_reviews_after(&self, after: i64, _limit: usize) -> Result<Vec<QueuedReview>> {
            self.0.set(self.0.get().max(after) + 1);
            Ok(vec![QueuedReview {
                id: self.0.get(),
                ..QueuedReview::default()
            }])
        }
    }

    /// Returns ids out of order within one page.
    struct Unordered;
    impl crate::store::ReviewStore for Unordered {
        fn pending_reviews_after(&self, _after: i64, _limit: usize) -> Result<Vec<QueuedReview>> {
            Ok([5, 3]
                .map(|id| QueuedReview {
                    id,
                    ..QueuedReview::default()
                })
                .to_vec())
        }
    }

    #[test]
    fn a_walk_that_never_ends_stops_at_the_page_cap_with_an_error() {
        let mut pages = PendingPages::new();
        let store = Endless(std::cell::Cell::new(0));
        let mut n = 0;
        let mut end = Ok(None);
        // bounded, so a missing cap fails this test by assertion and does not hang it
        for _ in 0..MAX_PAGES + 10 {
            end = pages.next(&store);
            match end {
                Ok(Some(_)) => n += 1,
                _ => break,
            }
        }
        assert!(end.is_err(), "the walk ended without an error");
        assert_eq!(n, MAX_PAGES);
    }

    #[test]
    fn a_page_out_of_order_is_an_error() {
        assert!(PendingPages::new().next(&Unordered).is_err());
    }

    #[test]
    fn the_pages_of_a_queue_cover_every_item_once_in_order() {
        let s = SqliteStore::open_in_memory().unwrap();
        for seq in 1..=(2 * DRAIN_LIMIT as i64 + 1) {
            s.enqueue_review(&NewReview::new("r", seq, "public", "x"), 3)
                .unwrap();
        }
        let mut pages = PendingPages::new();
        let (mut seen, mut calls) = (Vec::new(), 0);
        while let Some(page) = pages.next(&s).unwrap() {
            calls += 1;
            seen.extend(page.iter().map(|q| q.id));
        }
        assert_eq!((calls, seen.len()), (3, 2 * DRAIN_LIMIT + 1));
        assert!(seen.windows(2).all(|w| w[0] < w[1]));
    }

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

    /// Defers the first `n` seqs (as a screen that flags them does) and allows the rest.
    struct DefersFirst(i64);
    impl Judge for DefersFirst {
        fn judge(&self, item: &ReviewItem) -> Verdict {
            if item.seq <= self.0 {
                Verdict::Defer
            } else {
                Verdict::Allow
            }
        }
    }

    #[test]
    fn items_behind_a_full_page_of_deferred_ones_are_still_judged() {
        // #1736: the drain used to look at the oldest DRAIN_LIMIT items only, so 100 stuck ones starved the rest
        let s = SqliteStore::open_in_memory().unwrap();
        s.join("r", "alice", 1).unwrap();
        let total = DRAIN_LIMIT as i64 + 5;
        for seq in 1..=total {
            s.post(crate::store::Seat::new("r", "alice"), "hello", 2)
                .unwrap();
            s.enqueue_review(&NewReview::new("r", seq, "public", "crosses"), 3)
                .unwrap();
        }
        let out = drain(&s, &DefersFirst(DRAIN_LIMIT as i64), 10).unwrap();
        assert_eq!((out.allowed, out.deferred), (5, DRAIN_LIMIT));
        assert_eq!(s.pending_reviews(1000).unwrap().len(), DRAIN_LIMIT);
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

    /// Defers on purpose with a note, as the rules judge does for a screen hit.
    struct Flagging(&'static str);
    impl Judge for Flagging {
        fn judge(&self, _: &ReviewItem) -> Verdict {
            Verdict::Defer
        }
        fn judge_noted(&self, _: &ReviewItem) -> (Verdict, Option<String>) {
            (Verdict::Defer, Some(self.0.into()))
        }
    }

    #[test]
    fn a_deferral_note_is_kept_on_the_waiting_row_cleaned_and_replaced_by_the_verdict() {
        let s = queued();
        drain(&s, &Flagging("screen:injection\n<b>x</b>"), 10).unwrap();
        let q = &s.pending_reviews(10).unwrap()[0];
        assert_eq!(q.detail, "screen:injectionbxb");
        assert_eq!(flag_text(&"y".repeat(1000)).len(), FLAG_MAX);
        // a verdict replaces it, and a flag on a resolved row changes nothing
        let id = q.id;
        assert!(
            s.resolve_review(
                &crate::store::ReviewResolution::new(id, "deny", "by r: no"),
                11
            )
            .unwrap()
        );
        s.set_review_flag(id, "late").unwrap();
        assert_eq!(s.reviews(10).unwrap()[0].detail, "by r: no");
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
