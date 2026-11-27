//! The never-read count through the shared gateway's backend (#1415): `SharedBackend::first_post_unread` sends the call to
//! the owner thread as the caller, and gives the same answer the gateway gives in the calling thread.

use super::cancel_hook_tests::shared;
use super::{Backend, Session, SessionConfig};
use crate::domain::Principal;
use serde_json::json;

fn post(session: &Session<'_, crate::gateway::SharedBackend>, body: &str) {
    let frames = session.call("post", json!({"room": "r", "body": body}));
    assert!(
        frames
            .iter()
            .all(|f| !matches!(f, crate::protocol::ServerFrame::Response { ok: false, .. })),
        "{frames:?}"
    );
}

#[test]
fn the_shared_backend_counts_what_waits_once_for_a_first_post_and_never_again() {
    let (gw, _db) = shared("never-read-shared");
    let backend = gw.session_backend();
    let (alice, bob) = (
        Principal::unattributed("alice"),
        Principal::unattributed("bob"),
    );
    let config = SessionConfig::default();
    let (sa, sb) = (
        Session::over(&backend, &alice, &config),
        Session::over(&backend, &bob, &config),
    );
    post(&sa, "one");
    post(&sa, "two");
    post(&sb, "hello");
    assert_eq!(Backend::first_post_unread(&backend, &bob, "r").unwrap(), 2);
    assert_eq!(sb.first_post_unread("r"), 2);
    // the poster of the first message is not told about itself, and a second post is never a first
    assert_eq!(sa.first_post_unread("r"), 0);
    post(&sb, "again");
    assert_eq!(sb.first_post_unread("r"), 0);
}

#[test]
fn the_shared_backend_refuses_a_caller_the_guard_refuses_and_counts_nothing_for_it() {
    let (gw, _db) = shared("never-read-refused");
    let backend = gw.session_backend();
    let bad = Principal::unattributed("");
    assert!(Backend::first_post_unread(&backend, &bad, "r").is_err());
}
