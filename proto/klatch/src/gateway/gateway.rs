use crate::domain::{
    Claim, Error, MemberInfo, Message, PARKED_DEFAULT_SECS, Parked, Posted, Principal, Result,
    RoomDigest, parse_parked, valid_name,
};
use crate::gateway::{Clock, Guard, Operation};
use crate::store::{ClaimRequest, HookEvent, NewMessage, Release, Seat, Seq, Store};

/// What a hook needs: unread messages and others' claims per room, and the rooms that were refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digest {
    pub rooms: Vec<RoomDigest>,
    pub refused: Vec<Refused>,
    pub now: i64,
}

/// A room the guard would not let the caller read, with the reason. Shown, never silently dropped (R19).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub room: String,
    pub reason: String,
}

mod admin;
mod anchors;
mod github_anchor;
pub use anchors::{AnchorAdapter, valid_anchor};
use github_anchor::{normalize_anchor, parse_events};
mod budget;
mod crossing;
mod delivery;
pub use delivery::DeliveryView;
mod dm;
mod lease;
pub use lease::{
    DEFAULT_CAP_SECS, DEFAULT_EXTEND_SECS, DEFAULT_GRACE_SECS, LeaseAction, LeasePolicy,
    LeaseSource, Observed, lease_action,
};
mod grantor;
mod history;
pub use history::{History, HistoryRange};
mod facilitator;
mod integrator;
mod kind;
mod librarian;
mod policy;
mod reviewer;
mod roles;
pub use roles::Role;
mod rate;
pub use rate::RateLimitEntry;
mod room_require;
mod room_settings;
mod rooms;
mod runs;
pub use rooms::invite_text;
pub use runs::{Archive, RunInfo, run_prefix, valid_run_id};
mod scrub;
mod session_refusal;
mod ssh_key;
mod tripwire;
pub(crate) use tripwire::CANARY_KIND;
mod workers;
pub use workers::{
    CredentialInfo, CredentialSpec, DEFAULT_CREDENTIAL_MAX_LIFE_SECS, MAX_CREDENTIAL_TTL_SECS,
    check_credential,
};

mod board;
mod checks;
mod claims;
mod presence;
mod reading;
use checks::{
    check_event_name, check_member, check_no_credentials, check_path, check_reason, check_room,
    check_session_id, check_status, identifier, invalid, plain_text,
};
mod specs;
pub use specs::{ClaimSpec, Handoff};

#[cfg(test)]
pub(crate) trait AsSpy {
    fn spy_calls(&self) -> usize;
    fn spy_fail(&self, fail: bool);
}

/// What a post is: the room, the body and the label asked for (`None` for the room's own).
pub type PostArgs<'a> = (&'a str, &'a str, Option<&'a str>);

pub struct Gateway<S: Store> {
    store: S,
    clock: Box<dyn Clock>,
    guard: Box<dyn Guard>,
    notifier: Box<dyn crate::gateway::Notifier>,
    /// Built for members (#507): an admin act needs the admin agent role. See `roles.rs`.
    for_members: bool,
    /// Where the lease rules learn whether a holder is alive (#304); none means claims keep only their TTL.
    leases: Option<Box<dyn LeaseSource>>,
}

impl<S: Store> Gateway<S> {
    /// Panics when `store` does not declare every feature the gateway relies on (#485): a backend that skips one would
    /// otherwise lose it without any error. It is a defect of the backend, found the first time it is wired.
    pub fn new(store: S, clock: impl Clock + 'static, guard: impl Guard + 'static) -> Self {
        if let Err(e) = crate::store::require_features(&store) {
            panic!("{e}");
        }
        Self {
            store,
            clock: Box::new(clock),
            guard: Box::new(guard),
            notifier: Box::new(crate::gateway::NoNotifier),
            for_members: false,
            leases: None,
        }
    }

    /// Mark this gateway as the one members act through (`klatch act`, `review`, `serve` and the hooks): an admin
    /// act is then refused to any member that does not hold the admin agent role. The terminal and launcher
    /// gateways (the human's) are not built this way.
    pub fn for_members(mut self) -> Self {
        self.for_members = true;
        self
    }

    /// Tell `notifier` about every post that is stored (#72). Replaces the default, which tells nobody.
    pub fn with_notifier(mut self, notifier: impl crate::gateway::Notifier + 'static) -> Self {
        self.notifier = Box::new(notifier);
        self
    }

    pub fn now(&self) -> i64 {
        self.clock.now()
    }

    /// The store, for a test in another module that has to stage what no public call can yet.
    #[cfg(test)]
    pub(crate) fn store_for_tests(&self) -> &S {
        &self.store
    }

    #[cfg(test)]
    pub(crate) fn store_calls(&self) -> usize
    where
        S: AsSpy,
    {
        self.store.spy_calls()
    }

    #[cfg(test)]
    pub(crate) fn set_store_failing(&self, fail: bool)
    where
        S: AsSpy,
    {
        self.store.spy_fail(fail)
    }

    /// Join as a caller that named no harness: the generic floor (CLI and instructions file), so a room that
    /// requires a capability the floor lacks refuses it. See `join_with_harness`.
    pub fn join(&self, who: &Principal, room: &str) -> Result<()> {
        self.join_with_harness(who, room, &crate::harness::Generic)
            .map(|_| ())
    }

    pub fn post(&self, who: &Principal, room: &str, body: &str) -> Result<Posted> {
        self.post_labeled(who, (room, body, None))
    }

    /// Post under a label: the room's own unless `label` says a lower one. `post` is `(room, body, label)`.
    pub fn post_labeled(&self, who: &Principal, post: PostArgs) -> Result<Posted> {
        self.post_with(who, post, None)
    }

    /// Post a message that carries a handoff: a kind and, optionally, an opaque payload for the next member. The
    /// payload is held to every rule the body is (credentials, canaries, private terms, the label of the room) and is
    /// sealed with the body under a sealed label.
    pub fn post_handoff(
        &self,
        who: &Principal,
        post: PostArgs,
        handoff: &Handoff,
    ) -> Result<Posted> {
        self.post_with(who, post, Some(handoff))
    }

    fn post_with(
        &self,
        who: &Principal,
        (room, body, label): PostArgs,
        handoff: Option<&Handoff>,
    ) -> Result<Posted> {
        check_member(who)?;
        check_room(room)?;
        if let Some(h) = handoff {
            h.check()?;
        }
        if body.trim().is_empty() {
            return Err(Error::Invalid(
                "empty message: there is nothing to post".into(),
            ));
        }
        self.refuse_text(who, (room, "post"), body)?;
        if let Some(payload) = handoff.and_then(|h| h.payload) {
            self.refuse_text(who, (room, "post"), payload)?;
        }
        self.authorize(
            who,
            &Operation::Post {
                room: room.into(),
                body: body.into(),
            },
        )?;
        let created = self.check_creation(who, room)?;
        let label = self.post_label(who, room, label)?;
        self.check_budget(who, room, body)?;
        self.check_rate(who, room)?;
        let message = NewMessage::new((room, &who.member), who.attribution, &label, body);
        let message = match handoff {
            Some(Handoff {
                kind,
                payload: Some(p),
            }) => message.with_handoff(kind, p),
            Some(Handoff {
                kind,
                payload: None,
            }) => message.with_kind(kind),
            None => message,
        };
        let posted = self.store.post_as(&message, self.now())?;
        self.note_creation(who, room, created)?;
        self.notifier.posted(room, posted.seq, &who.member);
        self.park_by_post(who, body);
        self.store
            .clear_done(Seat::new(room, &who.member), self.now())?;
        Ok(posted)
    }

    /// The administrative rules (`klatch admin`, R11) and then the guard, before the store is touched: a member
    /// removed from a room, a paused gateway and a closed room each refuse with the rule that did.
    fn authorize(&self, who: &Principal, op: &Operation) -> Result<()> {
        self.authorize_quietly(who, op).inspect_err(|e| {
            self.note_refusal(who, (op.room().unwrap_or(""), op.name()), e);
        })
    }

    /// `authorize` without a refusal record, for the hook digest: it reports a room it cannot show on every run, and
    /// that is not an attempt by the member.
    fn authorize_quietly(&self, who: &Principal, op: &Operation) -> Result<()> {
        if let Some(room) = op.room() {
            self.check_stops(who, room, op)?;
            self.check_run(room)?;
            self.check_dm(who, room)?;
            if !matches!(op, Operation::Release { .. }) {
                self.check_clearance(who, room)?;
            }
        }
        self.check_guard(who, op)
    }

    /// Validate the caller and the room, then ask the guard. Runs before the store is touched.
    fn admit(&self, who: &Principal, room: &str, op: Operation) -> Result<()> {
        check_member(who)?;
        check_room(room)?;
        self.authorize(who, &op)
    }
}

#[cfg(test)]
mod tests;
