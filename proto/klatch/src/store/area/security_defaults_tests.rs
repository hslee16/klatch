//! What a store that implements only the required methods answers for the security-relevant ones (#485): an error,
//! never the answer "nothing is set, nobody is blocked, nothing was refused", which would let a check pass because
//! the backend cannot keep the thing the check reads.

use super::*;
use crate::domain::{Error, Result};
use crate::store::memory_spike::MemoryStore;
use crate::store::{NewAudit, NewRefusal};

fn unsupported<T: std::fmt::Debug>(r: Result<T>) -> bool {
    matches!(r, Err(Error::Backend(_)))
}

#[test]
fn policy_audit_and_refusals_are_errors_not_empty() {
    let s = MemoryStore::new();
    let audit = NewAudit::new("a", "t", "d");
    let refusal = NewRefusal::new("m", "r", "rule", "op");
    let outcomes: [(&str, bool); 9] = [
        ("policy_get", unsupported(s.policy_get("k"))),
        ("policy_set", unsupported(s.policy_set("k", Some("v"), 1))),
        ("policy_list", unsupported(s.policy_list("k"))),
        ("record_audit", unsupported(s.record_audit(&audit, 1))),
        ("audit_log", unsupported(s.audit_log(5))),
        ("record_refusal", unsupported(s.record_refusal(&refusal, 1))),
        ("refusals", unsupported(s.refusals(5))),
        ("refusals_of", unsupported(s.refusals_of("m", 5))),
        (
            "refusal_rules_since",
            unsupported(s.refusal_rules_since("m", 0)),
        ),
    ];
    for (name, ok) in outcomes {
        assert!(ok, "{name} answered instead of refusing to pretend");
    }
}

#[test]
fn blocks_removal_and_room_state_are_errors_not_empty() {
    let s = MemoryStore::new();
    let outcomes: [(&str, bool); 7] = [
        ("block_member", unsupported(s.block_member("r", "m", 1))),
        ("unblock_member", unsupported(s.unblock_member("r", "m"))),
        ("is_blocked", unsupported(s.is_blocked("r", "m"))),
        ("remove_member", unsupported(s.remove_member("r", "m", 1))),
        (
            "set_room_state",
            unsupported(s.set_room_state("r", Some("closed"), 1)),
        ),
        ("room_state", unsupported(s.room_state("r"))),
        (
            "delete_rooms_with_prefix",
            unsupported(s.delete_rooms_with_prefix("run.", 1)),
        ),
    ];
    for (name, ok) in outcomes {
        assert!(ok, "{name} answered instead of refusing to pretend");
    }
}

#[test]
fn the_post_counts_the_rate_limit_reads_are_an_error_not_zero() {
    let s = MemoryStore::new();
    assert!(unsupported(s.posts_since("m", 0)));
}

#[test]
fn the_message_room_lookup_is_an_error_not_none() {
    let s = MemoryStore::new();
    assert!(unsupported(s.message_room("any")));
}
