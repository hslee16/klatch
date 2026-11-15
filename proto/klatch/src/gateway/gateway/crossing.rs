//! Crossings (#497, DESIGN D4): the pipeline that makes the review queue real. A crossing is one thing only: a member
//! asks that a message in a higher-labelled room be copied into a lower-labelled one. Nothing else enqueues, so
//! nothing crosses by accident. The request is checked against the floors, queued as a pointer, and held until a
//! verdict (the judge or the reviewer) comes back; `allow` and `redact` then post the text into the target room and
//! `deny` posts nothing. The floors are checked again at release, so a term added between the request and the verdict
//! still stops it, and every outcome is audited without the text.

use super::*;
use crate::domain::Attribution;
use crate::gateway::judge::load_item;
use crate::store::{NewAudit, NewMessage, NewReview, QueuedReview};

/// What a call to `release_crossings` did to one item.
pub type Applied = (i64, &'static str);

const RELEASED: &str = "released";
const BLOCKED: &str = "blocked";

impl<S: Store> Gateway<S> {
    /// Ask for message `seq` of `from` to be copied into `to`. The caller must be able to read the source and post
    /// to the target, the target must be labelled strictly below the message, and the text must already pass the
    /// floors for the target. Returns the queue id; nothing moves until a verdict.
    pub fn request_crossing(&self, who: &Principal, from: &str, seq: i64, to: &str) -> Result<i64> {
        check_member(who)?;
        check_room(from)?;
        check_room(to)?;
        self.authorize(who, &Operation::read(from, false))?;
        self.authorize(who, &post_op(to))?;
        let probe = QueuedReview::pointer(from, seq);
        let Some(item) = load_item(&self.store, &probe)? else {
            return Err(Error::Invalid(format!("no message {seq} in room {from}")));
        };
        let label = self.message_label(from, seq)?;
        let to_label = self.room_label(to)?;
        let chain = self.chain()?;
        if chain.rank(&to_label) >= chain.rank(&label) {
            return Err(Error::Invalid(format!(
                "room {to} is labelled {to_label}, not below the message ({label}): post it there directly"
            )));
        }
        self.refuse_text(who, (to, "crossing"), &item.text)?;
        let new = NewReview::new(from, seq, &to_label, "crossing").crossing(to, &who.member);
        self.store.enqueue_review(&new, self.now())
    }

    /// The label the message was posted under.
    fn message_label(&self, room: &str, seq: i64) -> Result<String> {
        let found = self.store.tail(room, seq - 1, 1)?;
        let m = found.into_iter().find(|m| m.seq == seq);
        Ok(m.map(|m| m.label)
            .unwrap_or_else(|| self.room_label(room).unwrap_or_default()))
    }

    /// Apply every verdict that has not been applied: post allowed and redacted crossings into their target rooms,
    /// after checking the floors once more. Safe to run again; an item is applied once. Returns what happened to
    /// each item it touched.
    pub fn release_crossings(&self) -> Result<Vec<Applied>> {
        let mut out = Vec::new();
        for q in self.store.unapplied_reviews(crate::gateway::DRAIN_LIMIT)? {
            let outcome = self.apply_one(&q)?;
            self.store.mark_review_applied(q.id, outcome)?;
            self.audit_crossing(&q, outcome)?;
            out.push((q.id, outcome));
        }
        Ok(out)
    }

    fn apply_one(&self, q: &QueuedReview) -> Result<&'static str> {
        let Some(text) = self.text_to_release(q)? else {
            return Ok(BLOCKED);
        };
        let asker = Principal::unattributed(&q.requester);
        if self
            .refuse_text(&asker, (&q.to_room, "crossing"), &text)
            .is_err()
        {
            return Ok(BLOCKED);
        }
        let label = self.room_label(&q.to_room)?;
        let message = NewMessage::new(
            (&q.to_room, &q.requester),
            Attribution::Unattributed,
            &label,
            &text,
        );
        let posted = self.store.post_as(&message, self.now())?;
        self.notifier.posted(&q.to_room, posted.seq, &q.requester);
        Ok(RELEASED)
    }

    /// The text a verdict lets across: the redaction, or the original for `allow`; none if the original is gone.
    fn text_to_release(&self, q: &QueuedReview) -> Result<Option<String>> {
        if q.verdict == "redact" {
            return Ok(Some(q.redacted.clone()));
        }
        Ok(load_item(&self.store, q)?.map(|i| i.text))
    }

    fn audit_crossing(&self, q: &QueuedReview, outcome: &str) -> Result<()> {
        let target = format!("{}#{}", q.room, q.seq);
        let note = format!(
            "{outcome} to {} by {} ({})",
            q.to_room, q.requester, q.verdict
        );
        self.store
            .record_audit(&NewAudit::new("crossing", &target, &note), self.now())
    }

    /// Every queue item, newest first, with its verdict and what became of it. Pointers and outcomes, never text.
    pub fn review_queue(&self, who: &Principal, limit: usize) -> Result<Vec<QueuedReview>> {
        check_member(who)?;
        self.check_guard(who, &Operation::admin("review-queue", ""))?;
        let mut rows = self.store.reviews(limit)?;
        for r in &mut rows {
            r.redacted.clear();
        }
        Ok(rows)
    }
}

fn post_op(room: &str) -> Operation {
    Operation::Post {
        room: room.into(),
        body: String::new(),
    }
}
