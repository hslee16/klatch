//! Test doubles for the park and review conformance tests (#956). `Faulty` keeps park/unpark and the review queue on
//! top of the in-memory spike, correctly when its fault is `None` and with exactly one deliberate defect otherwise, so
//! each new conformance test is shown to pass on a sound store and to fail on a broken one.

use super::*;
use crate::domain::{Claim, HookRun, MemberInfo, Message, Parked, Posted};
use crate::store::conformance as c;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::panic::catch_unwind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    None,
    ParkLeavesClaimsAlone,
    ParkIsNotListed,
    SecondParkIsIgnored,
    UnparkKeepsTheStretch,
    UnparkIsNotListedAsEnded,
    UnparkOfAnUnparkedMemberCutsClaims,
    PendingKeepsResolved,
    SecondVerdictWins,
    SameMessageQueuesTwice,
    ListingIsOldestFirst,
    CrossingLosesItsRequester,
    DenyIsDueToApply,
    NoticesRepeat,
    FlagIsDropped,
    StatusIgnoresTheRoom,
    LastParentWins,
    HookRunsAreNotCounted,
}

thread_local! {
    static FAULT: std::cell::Cell<Fault> = const { std::cell::Cell::new(Fault::None) };
}

fn fault() -> Fault {
    FAULT.with(|f| f.get())
}

struct Faulty {
    inner: MemoryStore,
    parks: RefCell<BTreeMap<String, Parked>>,
    queue: RefCell<Vec<QueuedReview>>,
    redacted: RefCell<BTreeMap<i64, String>>,
    told: RefCell<BTreeMap<i64, u8>>,
    /// (room, member) -> (status, set at); the room is empty under `StatusIgnoresTheRoom`.
    statuses: RefCell<BTreeMap<(String, String), (String, i64)>>,
    parents: RefCell<BTreeMap<String, String>>,
    /// (member, event) -> the run so far
    hooks: RefCell<BTreeMap<(String, String), HookRun>>,
    fault: Fault,
}

impl Faulty {
    fn new() -> Self {
        Self {
            inner: MemoryStore::new(),
            parks: Default::default(),
            queue: Default::default(),
            redacted: Default::default(),
            told: Default::default(),
            statuses: Default::default(),
            parents: Default::default(),
            hooks: Default::default(),
            fault: fault(),
        }
    }

    /// The room a status is kept under: its own, or none at all when the status is wrongly shared across rooms.
    fn status_room<'a>(&self, room: &'a str) -> &'a str {
        if self.fault == Fault::StatusIgnoresTheRoom {
            ""
        } else {
            room
        }
    }

    /// Show the machine status kept for `m` under `room_key`, if there is one.
    fn overlay_status(&self, room_key: &str, m: &mut MemberInfo) {
        if let Some((text, at)) = self
            .statuses
            .borrow()
            .get(&(room_key.to_string(), m.name.clone()))
        {
            m.status = Some(text.clone());
            m.status_ts = *at;
        }
    }

    /// Sets every claim of `owner` live after `now` to what `expires` makes of it.
    fn reset_claims(&self, owner: &str, now: i64, expires: impl Fn(i64) -> i64) {
        for r in self.inner.rooms.borrow_mut().values_mut() {
            for (claim, exp) in r
                .claims
                .iter_mut()
                .filter(|(c, e)| c.owner == owner && *e > now)
            {
                *exp = expires(*exp);
                claim.expires = *exp;
            }
        }
    }
}

impl MessageStore for Faulty {
    fn join(&self, room: &str, member: &str, now: i64) -> Result<()> {
        self.inner.join(room, member, now)
    }
    fn post_as(&self, m: &NewMessage, now: i64) -> Result<Posted> {
        self.inner.post_as(m, now)
    }
    fn read(&self, at: Seat, advance: bool, now: i64) -> Result<Vec<Message>> {
        self.inner.read(at, advance, now)
    }
    fn tail(&self, room: &str, after_seq: i64, limit: usize) -> Result<Vec<Message>> {
        self.inner.tail(room, after_seq, limit)
    }
    fn head(&self, room: &str) -> Result<i64> {
        self.inner.head(room)
    }
    fn advance_cursor(&self, at: Seat, seq: Seq, now: i64) -> Result<()> {
        self.inner.advance_cursor(at, seq, now)
    }
}

impl MemberStore for Faulty {
    fn members(&self, room: &str) -> Result<Vec<MemberInfo>> {
        let mut out = self.inner.members(room)?;
        if self.fault != Fault::ParkIsNotListed {
            for m in &mut out {
                m.parked = self.parks.borrow().get(&m.name).cloned();
            }
        }
        let room_key = self.status_room(room);
        for m in &mut out {
            self.overlay_status(room_key, m);
            m.parent = self.parents.borrow().get(&m.name).cloned();
        }
        Ok(out)
    }

    fn rooms_of(&self, member: &str) -> Result<Vec<String>> {
        self.inner.rooms_of(member)
    }

    fn set_status(&self, at: Seat, status: &str, now: i64) -> Result<()> {
        self.inner.join(at.room, at.member, now)?;
        self.statuses.borrow_mut().insert(
            (self.status_room(at.room).into(), at.member.into()),
            (status.into(), now),
        );
        Ok(())
    }

    fn note_parent(&self, member: &str, parent: &str, _now: i64) -> Result<()> {
        let mut p = self.parents.borrow_mut();
        if self.fault == Fault::LastParentWins {
            p.insert(member.into(), parent.into());
        } else {
            p.entry(member.into()).or_insert_with(|| parent.into());
        }
        Ok(())
    }

    fn record_hook_run(&self, run: &HookEvent, now: i64) -> Result<()> {
        let mut h = self.hooks.borrow_mut();
        if run.event == "SessionStart" {
            for ((m, _), r) in h.iter_mut() {
                if m == run.member {
                    r.since_start = 0;
                }
            }
        }
        let e = h
            .entry((run.member.into(), run.event.into()))
            .or_insert_with(|| HookRun {
                event: run.event.into(),
                last_ts: now,
                runs: 0,
                last_bytes: 0,
                total_bytes: 0,
                max_bytes: 0,
                since_start: 0,
            });
        e.last_ts = now;
        e.runs = if self.fault == Fault::HookRunsAreNotCounted {
            1
        } else {
            e.runs + 1
        };
        e.last_bytes = run.bytes;
        e.total_bytes += run.bytes;
        e.max_bytes = e.max_bytes.max(run.bytes);
        e.since_start += run.bytes;
        Ok(())
    }

    fn hook_runs(&self, member: &str) -> Result<Vec<HookRun>> {
        Ok(self
            .hooks
            .borrow()
            .iter()
            .filter(|((m, _), _)| m == member)
            .map(|(_, r)| r.clone())
            .collect())
    }

    fn park(&self, member: &str, parked: &Parked, now: i64) -> Result<()> {
        let held = self.parks.borrow().contains_key(member);
        if !(held && self.fault == Fault::SecondParkIsIgnored) {
            self.parks
                .borrow_mut()
                .insert(member.into(), parked.clone());
        }
        if self.fault != Fault::ParkLeavesClaimsAlone {
            let back_by = parked.back_by;
            self.reset_claims(member, now, |e| e.max(back_by));
        }
        Ok(())
    }

    fn unpark(&self, member: &str, now: i64) -> Result<()> {
        let was = self.parks.borrow().contains_key(member);
        if self.fault != Fault::UnparkIsNotListedAsEnded {
            self.parks.borrow_mut().remove(member);
        }
        let cut = (was && self.fault != Fault::UnparkKeepsTheStretch)
            || self.fault == Fault::UnparkOfAnUnparkedMemberCutsClaims;
        if cut {
            let normal = now + crate::domain::CLAIM_DEFAULT_SECS;
            self.reset_claims(member, now, |e| e.min(normal));
        }
        Ok(())
    }
}

impl ClaimStore for Faulty {
    fn claim(&self, q: &ClaimRequest, now: i64) -> Result<()> {
        self.inner.claim(q, now)
    }
    fn release(&self, room: &str, owner: &str, path: &str) -> Result<()> {
        self.inner.release(room, owner, path)
    }
    fn claims(&self, room: &str, now: i64) -> Result<Vec<Claim>> {
        self.inner.claims(room, now)
    }
}

impl AskStore for Faulty {}
impl CredentialStore for Faulty {}
impl PolicyStore for Faulty {}

impl ReviewStore for Faulty {
    fn enqueue_review(&self, item: &NewReview, now: i64) -> Result<i64> {
        let mut q = self.queue.borrow_mut();
        if self.fault != Fault::SameMessageQueuesTwice
            && let Some(old) = q.iter().find(|r| {
                r.room == item.room
                    && r.seq == item.seq
                    && r.to_label == item.to_label
                    && r.verdict.is_empty()
            })
        {
            return Ok(old.id);
        }
        let id = q.len() as i64 + 1;
        q.push(QueuedReview {
            id,
            room: item.room.into(),
            seq: item.seq,
            to_label: item.to_label.into(),
            reason: item.reason.into(),
            ts: now,
            to_room: item.to_room.into(),
            requester: if self.fault == Fault::CrossingLosesItsRequester {
                String::new()
            } else {
                item.requester.into()
            },
            ..QueuedReview::default()
        });
        Ok(id)
    }

    fn pending_reviews(&self, limit: usize) -> Result<Vec<QueuedReview>> {
        Ok(self
            .queue
            .borrow()
            .iter()
            .filter(|r| r.verdict.is_empty() || self.fault == Fault::PendingKeepsResolved)
            .take(limit)
            .cloned()
            .collect())
    }

    fn resolve_review(&self, r: &ReviewResolution, _now: i64) -> Result<bool> {
        let mut q = self.queue.borrow_mut();
        let Some(item) = q.iter_mut().find(|x| x.id == r.id) else {
            return Ok(false);
        };
        if !item.verdict.is_empty() && self.fault != Fault::SecondVerdictWins {
            return Ok(false);
        }
        item.verdict = r.verdict.into();
        item.detail = r.detail.into();
        Ok(true)
    }

    fn set_review_flag(&self, id: i64, note: &str) -> Result<()> {
        if self.fault != Fault::FlagIsDropped
            && let Some(item) = self
                .queue
                .borrow_mut()
                .iter_mut()
                .find(|x| x.id == id && x.verdict.is_empty())
        {
            item.detail = note.into();
        }
        Ok(())
    }

    fn set_review_text(&self, id: i64, text: &str) -> Result<()> {
        self.redacted.borrow_mut().insert(id, text.into());
        Ok(())
    }

    fn reviews(&self, limit: usize) -> Result<Vec<QueuedReview>> {
        let mut all = self.queue.borrow().clone();
        if self.fault != Fault::ListingIsOldestFirst {
            all.reverse();
        }
        all.truncate(limit);
        Ok(all)
    }

    fn unapplied_reviews(&self, limit: usize) -> Result<Vec<QueuedReview>> {
        Ok(self
            .queue
            .borrow()
            .iter()
            .filter(|r| {
                !r.to_room.is_empty()
                    && r.applied.is_empty()
                    && (r.verdict == "allow"
                        || r.verdict == "redact"
                        || (r.verdict == "deny" && self.fault == Fault::DenyIsDueToApply))
            })
            .take(limit)
            .map(|r| QueuedReview {
                redacted: self
                    .redacted
                    .borrow()
                    .get(&r.id)
                    .cloned()
                    .unwrap_or_default(),
                ..r.clone()
            })
            .collect())
    }

    fn mark_review_applied(&self, id: i64, outcome: &str) -> Result<()> {
        if let Some(r) = self.queue.borrow_mut().iter_mut().find(|r| r.id == id) {
            r.applied = outcome.into();
        }
        Ok(())
    }

    fn crossing_notices(&self, room: &str, member: &str) -> Result<Vec<QueuedReview>> {
        let mut out = Vec::new();
        for r in self.queue.borrow().iter() {
            if !is_crossing_of(r, room, member) {
                continue;
            }
            let stage = notice_stage(r);
            let told = self.told.borrow().get(&r.id).copied().unwrap_or(0);
            if stage > told || self.fault == Fault::NoticesRepeat {
                self.told.borrow_mut().insert(r.id, stage);
                out.push(r.clone());
            }
        }
        Ok(out)
    }
}

/// Whether `r` is a review `member` asked in `room` that crosses to another room.
fn is_crossing_of(r: &QueuedReview, room: &str, member: &str) -> bool {
    r.room == room && r.requester == member && !r.to_room.is_empty()
}

/// How far a crossing review has got: 2 once it is denied or applied, else 1.
fn notice_stage(r: &QueuedReview) -> u8 {
    if r.verdict == "deny" || !r.applied.is_empty() {
        2
    } else {
        1
    }
}

impl RoomStore for Faulty {
    fn set_label_keys(&self, _keys: crate::keys::LabelKeys) {}
    fn features(&self) -> Features {
        Features::none()
            .with(Feature::MemberNotes)
            .with(Feature::Reviews)
    }
}

type Check = fn(fn() -> Faulty);

/// Each conformance test with the one defect that it must catch.
fn cases() -> Vec<(&'static str, Check, Fault)> {
    use Fault::*;
    vec![
        ("park holds claims", c::members::a_park_holds_live_claims_until_the_return_time_and_not_after::<Faulty>, ParkLeavesClaimsAlone),
        ("park is listed", c::members::a_parked_member_is_listed_with_why_and_return_time_until_unparked::<Faulty>, ParkIsNotListed),
        ("second park replaces", c::members::a_parked_member_is_listed_with_why_and_return_time_until_unparked::<Faulty>, SecondParkIsIgnored),
        ("unpark ends the park", c::members::a_parked_member_is_listed_with_why_and_return_time_until_unparked::<Faulty>, UnparkIsNotListedAsEnded),
        ("unpark cuts back", c::members::an_unpark_cuts_what_the_park_stretched_back_to_a_normal_lease::<Faulty>, UnparkKeepsTheStretch),
        ("unpark of an unparked member", c::members::an_unpark_of_a_member_that_is_not_parked_changes_nothing::<Faulty>, UnparkOfAnUnparkedMemberCutsClaims),
        ("pending only while pending", c::reviews::queued_items_come_back_oldest_first_and_only_while_pending::<Faulty>, PendingKeepsResolved),
        ("verdict once", c::reviews::a_verdict_is_recorded_once::<Faulty>, SecondVerdictWins),
        ("queued once per label", c::reviews::the_limit_bounds_one_drain_and_the_same_message_queues_once_per_label::<Faulty>, SameMessageQueuesTwice),
        ("listing newest first", c::reviews::the_listing_is_newest_first_and_keeps_resolved_items::<Faulty>, ListingIsOldestFirst),
        ("crossing keeps requester", c::reviews::a_crossing_item_keeps_its_target_requester_and_outcome::<Faulty>, CrossingLosesItsRequester),
        ("deny is never due", c::reviews::a_deny_or_a_plain_item_is_never_due_to_be_applied::<Faulty>, DenyIsDueToApply),
        ("notices told once", c::reviews::a_crossing_is_told_to_its_requester_once_while_waiting_and_once_for_its_outcome::<Faulty>, NoticesRepeat),
        ("flag kept", c::reviews::a_flag_is_kept_on_a_waiting_item_and_a_verdict_replaces_it::<Faulty>, FlagIsDropped),
    ]
}

fn run_with(fault: Fault, check: Check) -> bool {
    FAULT.with(|f| f.set(fault));
    let ok = catch_unwind(|| check(Faulty::new)).is_ok();
    FAULT.with(|f| f.set(Fault::None));
    ok
}

#[test]
fn every_new_conformance_test_passes_on_a_sound_store() {
    for (name, check, _) in cases() {
        assert!(
            run_with(Fault::None, check),
            "{name} fails on a sound store"
        );
    }
}

#[test]
fn every_new_conformance_test_fails_on_the_store_broken_in_the_way_it_guards() {
    for (name, check, fault) in cases() {
        assert!(!run_with(fault, check), "{name} did not catch {fault:?}");
    }
}

#[test]
fn the_feature_probe_runs_the_review_conformance_tests_and_catches_a_broken_declaration() {
    FAULT.with(|f| f.set(Fault::None));
    c::features::probe(Feature::Reviews, Faulty::new);
    FAULT.with(|f| f.set(Fault::SecondVerdictWins));
    let ran = catch_unwind(|| c::features::probe(Feature::Reviews, Faulty::new));
    FAULT.with(|f| f.set(Fault::None));
    assert!(
        ran.is_err(),
        "a broken review queue went unnoticed by the probe"
    );
}

#[test]
fn the_feature_probe_runs_the_member_notes_tests_and_catches_a_broken_park() {
    FAULT.with(|f| f.set(Fault::None));
    c::features::probe(Feature::MemberNotes, Faulty::new);
    for fault in [
        Fault::ParkLeavesClaimsAlone,
        Fault::StatusIgnoresTheRoom,
        Fault::LastParentWins,
        Fault::HookRunsAreNotCounted,
    ] {
        FAULT.with(|f| f.set(fault));
        let ran = catch_unwind(|| c::features::probe(Feature::MemberNotes, Faulty::new));
        FAULT.with(|f| f.set(Fault::None));
        assert!(ran.is_err(), "the probe did not catch {fault:?}");
    }
}
