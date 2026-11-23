//! Tests of the gateway, by topic.

use super::*;
use crate::domain::Error;
use crate::gateway::test_support::{SpyGuard, SpyStore};
use crate::gateway::{AllowAll, FixedClock, Operation};
use crate::store::areas::*;
use std::rc::Rc;

mod admin_acts;
mod admin_role;
mod anchors;
mod ask_refs;
mod ask_routing;
mod asks;
mod asks_notes;
mod board;
mod budget;
mod canary_surfaces;
mod claim_age;
mod claim_edges;
mod claims;
mod could_answer;
mod credential_renewal;
mod credentials;
mod crossing;
mod dead_holder;
mod delivery_view;
mod digest;
mod dm;
mod dm_nudge;
mod facilitator;
mod floors;
mod guarding;
mod handoff;
mod history;
mod invite_clearance;
mod labels;
mod lease;
mod lease_settings;
mod librarian;
mod machine_checkpoint;
mod machine_status;
mod member_card;
mod member_kind;
mod member_places;
mod member_surface;
mod members;
mod merge_claim;
mod merge_queue;
mod mute;
mod my_role;
mod notify;
mod own_refusals;
mod parked;
mod passive_claim;
mod possible_overwrite;
mod pr_lifecycle;
mod priority;
mod process_exited;
mod qa_correlation;
mod rate_limit;
mod receipt;
mod refusal_hints;
mod reserved_names;
mod reviewer;
mod role_order;
mod room_canary;
mod room_capability;
mod room_grantor;
mod room_require;
mod room_settings;
mod runs;
mod scrub;
mod ssh_key;
mod stream;
mod stuck;
mod tasks;
mod tripwire;
mod wake;
mod worker_credentials;
mod zero_trust;

pub(super) fn alice() -> Principal {
    Principal::unattributed("alice")
}
pub(super) fn bob() -> Principal {
    Principal::unattributed("bob")
}
pub(super) fn open() -> Gateway<SpyStore> {
    Gateway::new(SpyStore::new(), FixedClock(100), AllowAll)
}

pub(super) struct TestClock(pub(super) Rc<std::cell::Cell<i64>>);
impl Clock for TestClock {
    fn now(&self) -> i64 {
        self.0.get()
    }
}

pub(super) fn timed() -> (Gateway<SpyStore>, Rc<std::cell::Cell<i64>>) {
    let t = Rc::new(std::cell::Cell::new(1_000));
    (
        Gateway::new(SpyStore::new(), TestClock(t.clone()), AllowAll),
        t,
    )
}

pub(super) fn admin() -> Principal {
    Principal::unattributed("admin")
}

pub(super) fn rule_of(e: crate::domain::Error) -> String {
    match e {
        Error::Refused { rule, .. } => rule,
        other => panic!("not a refusal: {other}"),
    }
}

pub(super) fn is_invalid<T>(r: Result<T>) -> bool {
    matches!(r, Err(Error::Invalid(_)))
}

pub(super) fn is_backend<T>(r: Result<T>) -> bool {
    matches!(r, Err(Error::Backend(_)))
}

pub(super) fn is_refused<T>(r: Result<T>) -> bool {
    matches!(r, Err(Error::Refused { .. }))
}

/// Every named outcome must be true; a false one names itself.
pub(super) fn all_true(outcomes: &[(&str, bool)], context: &str) {
    for (name, ok) in outcomes {
        assert!(ok, "{name} {context}");
    }
}
