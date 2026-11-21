//! Zero trust classification, area: messages, reading and the room board.
//! How to add a method is in the comment of `zero_trust.rs`.

use super::*;

#[rustfmt::skip]
pub(super) const GUARDED: &[(&str, Call)] = &[
    ("join", |g, w| refused(g.join(w, "r"))),
    ("join_with_harness", |g, w| refused(g.join_with_harness(w, "r", &crate::harness::Generic))),
    ("post", |g, w| refused(g.post(w, "r", "x"))),
    ("post_labeled", |g, w| refused(g.post_labeled(w, ("r", "x", None)))),
    ("post_handoff", |g, w| refused(g.post_handoff(w, ("r", "x", None), &crate::gateway::Handoff {kind: "k", payload: None}))),
    ("dm", |g, w| refused(g.dm(w, "bob", "x", None))),
    ("read", |g, w| refused(g.read(w, "r", true))),
    ("tail", |g, w| refused(g.tail(w, "r", (0, 10)))),
    ("admin_tail_scan", |g, w| refused(g.admin_tail_scan(w, "r", (0, 10)))),
    ("history", |g, w| refused(g.history(w, "r", crate::gateway::HistoryRange::default()))),
    ("head", |g, w| refused(g.head(w, "r"))),
    ("ack", |g, w| refused(g.ack(w, "r", 1))),
    ("who", |g, w| refused(g.who(w, "r"))),
    ("receipt", |g, w| refused(g.receipt(w, "r", 1))),
    ("board", |g, w| refused(g.board(w, "r"))),
    ("open_asks", |g, w| refused(g.open_asks(w, "r"))),
    ("stuck", |g, w| refused(g.stuck(w, "r"))),
    ("member_card", |g, w| refused(g.member_card(w, "r", "bob"))),
    // another member's data
    ("hook_runs", |g, w| refused(g.hook_runs(w, "bob"))),
];

#[rustfmt::skip]
pub(super) const OWN_RECORD: &[(&str, &str)] = &[
    ("new", "builds the gateway"),
    ("with_notifier", "builds the gateway: who is told of a stored post"),
    ("for_members", "builds the gateway: members' admin acts need the admin agent role"),
    ("now", "the clock"),
    ("digest", "asks the guard once per room and reports the rooms it refused"),
    ("digest_with", "the same, with the backlog"),
    ("with_leases", "builds the gateway: where liveness comes from"),
];

#[rustfmt::skip]
pub(super) const NO_PRINCIPAL: &[&str] = &[
    "new",
    "with_notifier",
    "for_members",
    "now",
    "with_leases",
    // asks the guard once per room, so a principal whose credential ended is refused for every room (#161)
    "digest",
    "digest_with",
];

#[rustfmt::skip]
pub(super) const AFTER_END: &[(&str, OwnCall, Option<&str>)] = &[
];
