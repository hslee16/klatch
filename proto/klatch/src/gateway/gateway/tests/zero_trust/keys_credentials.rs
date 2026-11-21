//! Zero trust classification, area: keys, credentials, labels and clearances.
//! How to add a method is in the comment of `zero_trust.rs`.

use super::*;

#[rustfmt::skip]
pub(super) const GUARDED: &[(&str, Call)] = &[
    ("invite", |g, w| refused(g.invite(w, "bob", "r", "r"))),
    ("grant_room_clearance", |g, w| refused(g.grant_room_clearance(w, "r", "bob", "public"))),
    ("admin_set_room_clearance", |g, w| refused(g.admin_set_room_clearance(w, "r", "bob", Some("public")))),
    ("label_order", |g, w| refused(g.label_order(w))),
    ("admin_issue_credential", |g, w| refused(g.admin_issue_credential(w, &CredentialSpec {member: "w-1", clearance: "internal", ttl: 60, max_life: None, parent: None}))),
    ("admin_revoke_credential", |g, w| refused(g.admin_revoke_credential(w, "w-1"))),
    ("admin_credentials", |g, w| refused(g.admin_credentials(w))),
    ("admin_seal_label", |g, w| refused(g.admin_seal_label(w, "private"))),
    ("admin_set_room_label", |g, w| refused(g.admin_set_room_label(w, "r", "private"))),
    ("admin_set_clearance", |g, w| refused(g.admin_set_clearance(w, "bob", "private"))),
    ("admin_set_cap", |g, w| refused(g.admin_set_cap(w, "private"))),
    ("admin_set_label_chain", |g, w| refused(g.admin_set_label_chain(w, &["a", "b"]))),
    ("admin_set_room_require", |g, w| refused(g.admin_set_room_require(w, "r", Some("flag:wake")))),
    ("admin_plant_canary", |g, w| refused(g.admin_plant_canary(w, "r"))),
    ("admin_retire_canary", |g, w| refused(g.admin_retire_canary(w, "r"))),
];

#[rustfmt::skip]
pub(super) const OWN_RECORD: &[(&str, &str)] = &[
    ("note_session", "the caller's own session id"),
    ("note_member_fixed", "keeps a refusal for the caller itself, with no text the peer chose; it grants nothing and reads nothing"),
    ("record_hook_run", "the caller's own hook run record"),
    ("sealed_labels", "a policy lookup the CLI uses to find which keys to load; takes no principal"),
    ("sealed_labels_within", "the same, limited to what one member's clearance reaches; takes no principal"),
    ("end_credential", "a worker revokes only its own credential, and only as a verified principal"),
    ("verify_credential", "authentication itself: it takes proof of identity instead of a principal, and records a failure"),
    ("use_label_keys", "the keys this process holds, in memory only"),
    ("clearance_of", "a policy lookup the gateway uses on every call; takes no principal"),
    ("room_label", "a policy lookup the gateway uses on every call; takes no principal"),
];

#[rustfmt::skip]
pub(super) const NO_PRINCIPAL: &[&str] = &[
    "sealed_labels",
    "sealed_labels_within",
    "verify_credential",
    "use_label_keys",
    "clearance_of",
    "room_label",
];

#[rustfmt::skip]
pub(super) const AFTER_END: &[(&str, OwnCall, Option<&str>)] = &[
    ("note_session", |g, w| g.note_session(w, "sess-1"), None),
    ("note_member_fixed", |g, w| { g.note_member_fixed(w); Ok(()) }, Some("it only keeps a refusal row about the caller; a session whose credential ended has the same refusal to keep, and the row grants no authority")),
    ("record_hook_run", |g, w| g.record_hook_run(w, "SessionEnd", 0), Some("only the `SessionEnd` event: a session whose credential idled out must still be able to record that it ended; any other event is refused (checked separately)")),
    ("end_credential", |g, w| g.end_credential(w).map(|_| ()), Some("it only revokes the caller's own credential and does nothing when none is live, so a second call adds no authority")),
];
