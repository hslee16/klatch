//! Zero trust classification, area: crossings, review and the scrub rules.
//! How to add a method is in the comment of `zero_trust.rs`.

use super::*;

#[rustfmt::skip]
pub(super) const GUARDED: &[(&str, Call)] = &[
    ("request_crossing", |g, w| refused(g.request_crossing(w, "r", 1, "r"))),
    ("scrub_check", |g, w| refused(g.scrub_check(w, "x"))),
    ("admin_refusals", |g, w| refused(g.admin_refusals(w, 5))),
    ("admin_add_scrub_term", |g, w| refused(g.admin_add_scrub_term(w, "n", "t"))),
    ("admin_remove_scrub_term", |g, w| refused(g.admin_remove_scrub_term(w, "n"))),
    ("admin_scrub_terms", |g, w| refused(g.admin_scrub_terms(w))),
    ("review_queue", |g, w| refused(g.review_queue(w, 5))),
    ("review_next", |g, w| refused(g.review_next(w))),
    ("review_verdict", |g, w| refused(g.review_verdict(w, 1, &crate::domain::Verdict::Allow))),
];

#[rustfmt::skip]
pub(super) const OWN_RECORD: &[(&str, &str)] = &[
    ("my_refusals", "the caller's own refusal record: it reads only rows of the caller's principal name, so it is the same as reading its own hook record, and a member stopped by the breaker must still read why (#678)"),
    ("my_crossings", "the caller's own crossing requests: it reads only queue rows whose requester is the caller and returns a state and a fixed sentence, never the text, a redaction, a reviewer's reason or a scrub rule's name (#516)"),
    ("release_crossings", "applies only verdicts a reviewer or judge already recorded, with the floors checked again; it takes no principal and adds no authority. `klatch judge run --once` reaches it behind the launcher heuristic only, a gap kept in proto/AUTHZ-AUDIT.md"),
];

#[rustfmt::skip]
pub(super) const NO_PRINCIPAL: &[&str] = &[
    "release_crossings",
];

#[rustfmt::skip]
pub(super) const AFTER_END: &[(&str, OwnCall, Option<&str>)] = &[
    ("my_refusals", |g, w| g.my_refusals(w, 5).map(|_| ()), None),
    ("my_crossings", |g, w| g.my_crossings(w, 5).map(|_| ()), None),
];
