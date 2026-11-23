//! Zero trust classification, area: delivery, waking, muting and the limits on posting.
//! How to add a method is in the comment of `zero_trust.rs`.

use super::*;

#[rustfmt::skip]
pub(super) const GUARDED: &[(&str, Call)] = &[
    ("set_muted", |g, w| refused(g.set_muted(w, "r", true))),
    ("wait_poll", |g, w| refused(g.wait_poll(w, "r"))),
    ("wait_seen", |g, w| refused(g.wait_seen(w, "r", 1))),
    ("wake_mark", |g, w| refused(g.wake_mark(w, "r"))),
    ("wakeable", |g, w| refused(g.wakeable(w, "r"))),
    ("wake_summary", |g, w| refused(g.wake_summary(w, "r"))),
    ("outside_view", |g, w| refused(g.outside_view(w, "r"))),
    ("admin_set_room_stale", |g, w| refused(g.admin_set_room_stale(w, "r", Some(120)))),
    ("admin_set_room_ask_lease", |g, w| refused(g.admin_set_room_ask_lease(w, "r", Some(120)))),
    ("admin_set_rate_limit", |g, w| refused(g.admin_set_rate_limit(w, Some((5, 60))))),
    ("admin_rate_limits", |g, w| refused(g.admin_rate_limits(w))),
    ("admin_set_member_rate_limit", |g, w| refused(g.admin_set_member_rate_limit(w, "bob", Some((5, 60))))),
    ("admin_set_lease_cap", |g, w| refused(g.admin_set_lease_cap(w, Some(60)))),
    ("admin_set_lease_grace", |g, w| refused(g.admin_set_lease_grace(w, Some(60)))),
];

#[rustfmt::skip]
pub(super) const OWN_RECORD: &[(&str, &str)] = &[
    ("change_token", "when to look, never what: a number that moves when any connection commits, with no room, member or text in it. It takes no principal and returns nothing a peer sees; a session or wait uses it only to decide when to run the checked poll (#616)"),
    ("record_stop", "the caller's own park, from the end of its own turn"),
    ("sign_of_life", "the caller's own park, ended by its own hook"),
    ("note_tool", "the caller's own working trail: a class word and a plain repo-relative path, anything else refused (#1408)"),
    ("write_machine_checkpoint", "the caller's own checkpoint, written only into rooms the caller can read (it reads each through the guarded `board`) and where it works (#1408)"),
    ("set_machine_status", "the caller's own status, set only to a closed class word (working, idle, parked), only in rooms the caller can read and may set a status in, and never over a status the member set itself (#1403)"),
    ("lease_policy", "a policy lookup the lease rules use; takes no principal"),
    ("rate_limit", "a policy lookup the budget check uses on every post; takes no principal"),
    ("rate_limit_for", "the same lookup for one member's own limit (#972); takes no principal and changes nothing"),
    ("delivery_view", "the caller's own delivery view, a key built from the caller's own name"),
    ("set_delivery_view", "the caller's own delivery view, a key built from the caller's own name"),
];

#[rustfmt::skip]
pub(super) const NO_PRINCIPAL: &[&str] = &[
    "change_token",
    "lease_policy",
    "rate_limit",
    "rate_limit_for",
];

#[rustfmt::skip]
pub(super) const AFTER_END: &[(&str, OwnCall, Option<&str>)] = &[
    ("record_stop", |g, w| g.record_stop(w, 1, 0), None),
    ("sign_of_life", |g, w| g.sign_of_life(w), None),
    ("note_tool", |g, w| g.note_tool(w, "edit", Some("a.rs")), None),
    ("write_machine_checkpoint", |g, w| g.write_machine_checkpoint(w, None).map(|_| ()), None),
    ("set_machine_status", |g, w| g.set_machine_status(w, "working").map(|_| ()), None),
    ("delivery_view", |g, w| g.delivery_view(w).map(|_| ()), None),
    ("set_delivery_view", |g, w| g.set_delivery_view(w, crate::gateway::DeliveryView::Scope), None),
];
