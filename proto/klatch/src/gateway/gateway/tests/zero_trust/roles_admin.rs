//! Zero trust classification, area: roles and the administrative acts.
//! How to add a method is in the comment of `zero_trust.rs`.

use super::*;

#[rustfmt::skip]
pub(super) const GUARDED: &[(&str, Call)] = &[
    ("admin_set_room_grantor", |g, w| refused(g.admin_set_room_grantor(w, "r", "bob", true))),
    ("prune_members", |g, w| refused(g.prune_members(w, 0))),
    // the administrative acts
    // admin_close_room closes room "r": a closed room refuses before the guard is asked, so this area runs last in
    // the order of `zero_trust.rs` and this call stays at the head of the closing ones
    ("admin_close_room", |g, w| refused(g.admin_close_room(w, "r", true))),
    ("admin_pause", |g, w| refused(g.admin_pause(w, true))),
    ("admin_remove_member", |g, w| refused(g.admin_remove_member(w, "r", "bob"))),
    ("admin_allow_member", |g, w| refused(g.admin_allow_member(w, "r", "bob"))),
    ("admin_audit", |g, w| refused(g.admin_audit(w, 5))),
    ("admin_run_create", |g, w| refused(g.admin_run_create(w, "a"))),
    ("admin_run_end", |g, w| refused(g.admin_run_end(w, "a", None))),
    ("admin_runs", |g, w| refused(g.admin_runs(w))),
    ("admin_note", |g, w| refused(g.admin_note(w, ("key-test", "x"), "y"))),
    // found by the directory scan (#161): these files were not in the hand-kept list before
    ("admin_set_librarian", |g, w| refused(g.admin_set_librarian(w, "bob", true))),
    ("admin_set_reviewer", |g, w| refused(g.admin_set_reviewer(w, "bob", true))),
    ("admin_set_facilitator", |g, w| refused(g.admin_set_facilitator(w, "bob", true))),
    ("admin_set_integrator", |g, w| refused(g.admin_set_integrator(w, "bob", true))),
    ("admin_set_ssh_key", |g, w| refused(g.admin_set_ssh_key(w, "laptop", "bob", true))),
    ("admin_set_admin_agent", |g, w| refused(g.admin_set_admin_agent(w, "bob", true))),
    ("admin_set_room_creation_granted", |g, w| refused(g.admin_set_room_creation_granted(w, true))),
    ("admin_grant_room_capability", |g, w| refused(g.admin_grant_room_capability(w, "bob", true))),
];

#[rustfmt::skip]
pub(super) const OWN_RECORD: &[(&str, &str)] = &[
    ("room_grantor", "a policy lookup that takes no principal and returns one member name for a room (#893); the decision to grant reads it inside `grant_room_clearance`, which is GUARDED"),
    ("note_member_mode", "the caller's own permission mode, a short word with no content"),
    ("scope_of", "the caller's own scope"),
    ("my_role", "the caller's own role, one fixed word (#1090)"),
    ("set_scope", "the caller's own scope"),
    ("set_kind", "the caller's own kind, display only"),
    ("kind_of", "the caller's own kind"),
    ("rooms", "the rooms the caller joined"),
    ("role_of", "a policy lookup the gateway uses on every call; takes no principal"),
    ("is_admin_agent", "a role lookup; takes no principal"),
    ("is_librarian", "a role lookup; takes no principal"),
    ("is_facilitator", "a role lookup; takes no principal"),
    ("can_create_rooms", "a policy lookup `check_room_capability` uses on every call; takes no principal"),
];

#[rustfmt::skip]
pub(super) const NO_PRINCIPAL: &[&str] = &[
    "room_grantor",
    "role_of",
    "is_admin_agent",
    "is_librarian",
    "is_facilitator",
    "can_create_rooms",
];

#[rustfmt::skip]
pub(super) const AFTER_END: &[(&str, OwnCall, Option<&str>)] = &[
    ("note_member_mode", |g, w| g.note_member_mode(w, "default"), None),
    ("set_scope", |g, w| g.set_scope(w, "repo"), None),
    ("scope_of", |g, w| g.scope_of(w).map(|_| ()), None),
    ("my_role", |g, w| g.my_role(w).map(|_| ()), None),
    ("set_kind", |g, w| g.set_kind(w, Some(crate::domain::MemberKind::Bot)), None),
    ("kind_of", |g, w| g.kind_of(w).map(|_| ()), None),
    ("rooms", |g, w| g.rooms(w).map(|_| ()), None),
];
