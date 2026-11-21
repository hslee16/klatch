//! Zero trust classification, area: claims, anchors and the work they stand for.
//! How to add a method is in the comment of `zero_trust.rs`.

use super::*;
use crate::gateway::ClaimSpec;

#[rustfmt::skip]
pub(super) const GUARDED: &[(&str, Call)] = &[
    ("claim", |g, w| refused(g.claim(w, "r", &ClaimSpec::new("p", 10, "")))),
    ("process_exited", |g, w| refused(g.process_exited(w, "r", "bob"))),
    ("claim_ttl_for", |g, w| refused(g.claim_ttl_for(w, ("r", "ask:1"), None))),
    ("claims", |g, w| refused(g.claims(w, "r"))),
    ("could_answer", |g, w| refused(g.could_answer(w, "r", "bob", 1))),
    ("cycle_notices", |g, w| refused(g.cycle_notices(w, "r"))),
    ("release", |g, w| refused(g.release(w, "r", "p"))),
    ("release_with_note", |g, w| refused(g.release_with_note(w, ("r", "p"), "n"))),
    ("last_release", |g, w| refused(g.last_release(w, "r", "p"))),
    ("set_status", |g, w| refused(g.set_status(w, "r", "done"))),
    ("anchor_done", |g, w| refused(g.anchor_done(w, "wo:1"))),
    ("github_events", |g, w| refused(g.github_events(w, r#"{"number":1,"state":"closed","repository":{"full_name":"a/b"}}"#))),
    ("reap_anchors", |g, w| refused(g.reap_anchors(w, &NoFactory))),
];

#[rustfmt::skip]
pub(super) const OWN_RECORD: &[(&str, &str)] = &[
    ("note_parent", "the caller's own parent link"),
    ("release_all", "releases only the claims the caller holds"),
];

#[rustfmt::skip]
pub(super) const NO_PRINCIPAL: &[&str] = &[
];

#[rustfmt::skip]
pub(super) const AFTER_END: &[(&str, OwnCall, Option<&str>)] = &[
    ("note_parent", |g, w| g.note_parent(w, Some("lead")), None),
    ("release_all", |g, w| g.release_all(w).map(|_| ()), Some("frees only the caller's own claims, and the end of a session must free what it held even when the credential idled out first (#277); refusing would leave the claims held until their leases run out")),
];
