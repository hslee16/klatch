//! Crossings (#497, DESIGN D4): the pipeline that makes the review queue real. A crossing is one thing only: a member
//! asks that a message in a higher-labelled room be copied into a lower-labelled one. Nothing else enqueues, so
//! nothing crosses by accident. The request is checked against the floors, queued as a pointer, and held until a
//! verdict (the judge or the reviewer) comes back; `allow` and `redact` then post the text into the target room and
//! `deny` posts nothing. The floors are checked again at release, so a term added between the request and the verdict
//! still stops it, and every outcome is audited without the text.

use super::*;
use crate::domain::{Attribution, CrossingStatus, NEXT_STEP};
use crate::gateway::judge::load_item;
use crate::store::{NewAudit, NewMessage, NewReview, QueuedReview};

/// What a call to `release_crossings` did to one item.
pub type Applied = (i64, &'static str);

/// How many crossings one member may have waiting for a verdict at once (#1736). Raisable or lowerable by the human
/// through the policy key `crossing:cap` (`admin_set_crossing_cap`).
pub(super) const DEFAULT_CROSSING_CAP: usize = 20;
/// The most the cap may be set to.
const MAX_CROSSING_CAP: usize = 10_000;
/// A member is told of the cap in the refusal record once a minute.
const CAP_REFUSAL_WINDOW_SECS: i64 = 60;
const CAP_KEY: &str = "crossing:cap";
const RELEASED: &str = "released";
const BLOCKED: &str = "blocked";
/// The message kind of a released crossing (#679); its payload is `room#seq` of the source.
pub const CROSSED_KIND: &str = "crossed";

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
        if !self.store.rooms_of(&who.member)?.iter().any(|r| r == to) {
            return Err(Error::Invalid(format!(
                "you have not joined {to}: join it first (klatch join {to}), then ask again"
            )));
        }
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
        // counted and queued in one store transaction, so concurrent processes of one member cannot both take the
        // last place
        let cap = self.crossing_cap()?;
        match self.store.enqueue_review_capped(&new, self.now(), cap)? {
            Some(id) => Ok(id),
            None => Err(self.crossing_cap_refusal(who, from, cap)),
        }
    }

    /// The cap on one member's waiting crossings: the human's setting, else the default. A stored value that does not
    /// parse or is out of range is ignored, so the cap stays in force.
    pub(super) fn crossing_cap(&self) -> Result<usize> {
        Ok(self
            .store
            .policy_get(CAP_KEY)?
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|n| (1..=MAX_CROSSING_CAP).contains(n))
            .unwrap_or(DEFAULT_CROSSING_CAP))
    }

    /// Only the human: set how many crossings one member may have waiting (1 to 10000), or with `None` put the default
    /// back (#1736).
    pub fn admin_set_crossing_cap(&self, who: &Principal, cap: Option<usize>) -> Result<()> {
        if cap.is_some_and(|n| !(1..=MAX_CROSSING_CAP).contains(&n)) {
            return Err(Error::Invalid(format!(
                "the crossing cap is 1 to {MAX_CROSSING_CAP} waiting crossings per member, or off for the default"
            )));
        }
        let target = cap.map_or("off".to_string(), |n| n.to_string());
        self.administer(who, ("crossing-cap", &target), |now| {
            self.store
                .policy_set(CAP_KEY, cap.map(|n| n.to_string()).as_deref(), now)?;
            Ok(String::new())
        })
    }

    /// The refusal for a requester at the cap: it names the number and what frees a place, never a message or its
    /// text. It is kept as a refusal record (not a safety rule, so it does not trip the breaker), at most one row a
    /// minute per member however many requests are refused, so a loop of requests cannot grow the table; every
    /// request is still refused.
    fn crossing_cap_refusal(&self, who: &Principal, from: &str, cap: usize) -> Error {
        let e = Error::Refused {
            rule: "crossing-cap".into(),
            reason: format!(
                "you already have {cap} crossings waiting for a verdict, the most allowed; {NEXT_STEP}wait for a verdict on one (klatch crossings), then ask again"
            ),
        };
        let now = self.now();
        let told = self
            .store
            .refusal_rules_since(&who.member, now - CAP_REFUSAL_WINDOW_SECS)
            .is_ok_and(|rules| rules.iter().any(|r| r == "crossing-cap"));
        if !told {
            self.note_refusal(who, (from, "crossing"), &e);
        }
        e
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
    /// each item it touched. It is made on behalf of `who` (#836): the launcher's judge, never nobody. It is an admin
    /// act (`release-crossings`) that goes through the role check and the guard, and a refusal is kept; only the
    /// human at a terminal has it (no role holder's list names it). Each audit row says who released it.
    pub fn release_crossings(&self, who: &Principal) -> Result<Vec<Applied>> {
        check_member(who)?;
        self.check_guard(who, &Operation::admin("release-crossings", ""))
            .inspect_err(|e| {
                // the admin-scope refusal is already noted by the role check
                if !matches!(e, Error::Refused { rule, .. } if rule == "admin-scope") {
                    self.note_refusal(who, ("", "release-crossings"), e);
                }
            })?;
        self.release_pending(&who.member)
    }

    /// The release itself, for a caller that has already been guarded: `by` is recorded as the releaser.
    pub(super) fn release_pending(&self, by: &str) -> Result<Vec<Applied>> {
        let mut out = Vec::new();
        for q in self.store.unapplied_reviews(crate::gateway::DRAIN_LIMIT)? {
            let outcome = self.apply_one(&q)?;
            self.store.mark_review_applied(q.id, &outcome)?;
            let word = if outcome == RELEASED {
                RELEASED
            } else {
                BLOCKED
            };
            self.audit_crossing(&q, word, by)?;
            out.push((q.id, word));
        }
        Ok(out)
    }

    fn apply_one(&self, q: &QueuedReview) -> Result<String> {
        let Some(text) = self.text_to_release(q)? else {
            return Ok(blocked("gone"));
        };
        let asker = Principal::unattributed(&q.requester);
        if let Err(e) = self.refuse_text(&asker, (&q.to_room, "crossing"), &text) {
            return Ok(blocked(category_of(&e)));
        }
        let label = self.room_label(&q.to_room)?;
        // the handoff names the source by room and sequence number, never by its text (#679)
        let source = format!("{}#{}", q.room, q.seq);
        let message = NewMessage::new(
            (&q.to_room, &q.requester),
            Attribution::Unattributed,
            &label,
            &text,
        )
        .with_handoff(CROSSED_KIND, &source);
        let posted = self.store.post_as(&message, self.now())?;
        self.notifier.posted(&q.to_room, posted.seq, &q.requester);
        Ok(RELEASED.into())
    }

    /// The text a verdict lets across: the redaction, or the original for `allow`; none if the original is gone.
    fn text_to_release(&self, q: &QueuedReview) -> Result<Option<String>> {
        if q.verdict == "redact" {
            return Ok(Some(q.redacted.clone()));
        }
        Ok(load_item(&self.store, q)?.map(|i| i.text))
    }

    fn audit_crossing(&self, q: &QueuedReview, outcome: &str, by: &str) -> Result<()> {
        let target = format!("{}#{}", q.room, q.seq);
        let note = format!(
            "{outcome} to {} by {} ({}); applied by {by}",
            q.to_room, q.requester, q.verdict
        );
        self.store
            .record_audit(&NewAudit::new("crossing", &target, &note), self.now())
    }

    /// The caller's own crossings, newest first, each with its state and a fixed reason (#516). It reads only the
    /// caller's rows and takes no guard, like `my_refusals`: a member stopped by the breaker can still learn why.
    /// A deny reason, a redaction, the text and the name of a private term never appear.
    pub fn my_crossings(&self, who: &Principal, limit: usize) -> Result<Vec<CrossingStatus>> {
        check_member(who)?;
        self.check_own_record_live(who)?;
        let rows = self.store.reviews(crate::gateway::DRAIN_LIMIT)?;
        Ok(rows
            .iter()
            .filter(|q| q.requester == who.member)
            .take(limit)
            .map(crossing_status)
            .collect())
    }

    /// What the caller has not yet been told about its crossings that start in `room` (#679): `waiting` once after
    /// the request, then the outcome once. It reads only the caller's rows and posts nothing anywhere.
    pub(crate) fn crossing_notices(&self, room: &str, member: &str) -> Result<Vec<CrossingStatus>> {
        Ok(self
            .store
            .crossing_notices(room, member)?
            .iter()
            .map(crossing_status)
            .collect())
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

/// The applied word a blocked crossing is stored under: `blocked:<category>`, a category and never a rule name.
fn blocked(category: &str) -> String {
    format!("{BLOCKED}:{category}")
}

/// The kind of floor that stopped a crossing, from the refusal's rule. The scrub rule's own name and its term stay
/// with the human: the sender learns only that a private term matched.
fn category_of(e: &Error) -> &'static str {
    match e {
        Error::Refused { rule, .. } if rule.starts_with("scrub:") => "private-term",
        Error::Refused { rule, .. } if rule == "canary" => "canary",
        Error::Refused { rule, .. } if rule.starts_with("credential") => "credential",
        _ => "floor",
    }
}

fn crossing_status(q: &QueuedReview) -> CrossingStatus {
    let (state, why) = status_of(q);
    CrossingStatus {
        id: q.id,
        room: q.room.clone(),
        seq: q.seq,
        to_room: q.to_room.clone(),
        state: state.into(),
        why: why.into(),
    }
}

/// The state and the fixed reason for one queue row, from its verdict and what became of it.
fn status_of(q: &QueuedReview) -> (&'static str, &'static str) {
    let category = q.applied.strip_prefix("blocked:");
    match (q.verdict.as_str(), q.applied.as_str(), category) {
        ("", ..) => ("waiting", ""),
        ("deny", ..) => (
            "denied",
            "the reviewer decided this message should not cross to the lower room; ask a human if it is needed there",
        ),
        (_, "released", _) => ("released", ""),
        (_, _, Some("private-term")) => (
            "blocked",
            "the text matches a private-term rule that keeps it out of the lower room (private-term); reword the part that matches and ask again",
        ),
        (_, _, Some("credential")) => (
            "blocked",
            "the text looks like a credential, which never crosses (credential); remove it and ask again",
        ),
        (_, _, Some("canary")) => (
            "blocked",
            "the text carries a planted marker that must not leave its room (canary)",
        ),
        (_, _, Some("gone")) => ("blocked", "the message is no longer there to copy (gone)"),
        (_, _, Some(_)) => (
            "blocked",
            "a safety rule stopped the text when it was released (floor); reword it and ask again",
        ),
        _ => ("approved", "approved; waiting to be released"),
    }
}

fn post_op(room: &str) -> Operation {
    Operation::Post {
        room: room.into(),
        body: String::new(),
    }
}
