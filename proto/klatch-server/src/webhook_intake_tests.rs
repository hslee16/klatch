//! Tests of the request handler without a socket (`webhook.rs`, #1058, #1089, #1103): routing, the signature before
//! anything else, the delivery id and content de-duplication, and what is queued.

use crate::loopback::{Handler, Head, Request, Response};
use crate::webhook::*;
use crate::webhook_tests::{fixture, hex};

// ---- the handler (no socket) ----

fn head(method: &str, path: &str, sig: Option<&str>, event: Option<&str>) -> Head {
    let mut headers = Vec::new();
    if let Some(s) = sig {
        headers.push(("x-hub-signature-256".to_string(), s.to_string()));
    }
    if let Some(e) = event {
        headers.push(("x-github-event".to_string(), e.to_string()));
    }
    Head {
        method: method.into(),
        path: path.into(),
        headers,
        content_length: None,
    }
}

fn signed(secret: &[u8], body: &[u8]) -> String {
    format!("sha256={}", hex(&signature_digest(secret, body)))
}

fn intake(queue: &Queue) -> Intake<'_> {
    Intake::new(Secret::new(b"topsecret".to_vec()).unwrap(), queue)
}

#[test]
fn routing_is_one_path_and_post() {
    let q = Queue::new(8);
    let mut i = intake(&q);
    assert_eq!(i.route(&head("POST", "/github", None, None)), Ok(()));
    assert_eq!(
        i.route(&head("POST", "/other", None, None))
            .unwrap_err()
            .status,
        404
    );
    let r = i.route(&head("GET", "/github", None, None)).unwrap_err();
    assert_eq!((r.status, r.allow), (405, Some("POST")));
}

#[test]
fn a_correctly_signed_event_is_queued_and_answered_200() {
    let q = Queue::new(8);
    let mut i = intake(&q);
    let body = fixture("pull_request_opened");
    let r = i.handle(Request {
        head: head(
            "POST",
            "/github",
            Some(&signed(b"topsecret", &body)),
            Some("pull_request"),
        ),
        body,
    });
    assert_eq!(r.status, 200);
    let (got, _) = q.drain();
    assert_eq!(got.len(), 1);
}

#[test]
fn a_missing_or_wrong_signature_is_401_and_nothing_is_queued() {
    let q = Queue::new(8);
    let mut i = intake(&q);
    let body = fixture("pull_request_opened");
    for sig in [
        None,
        Some(signed(b"wrong", &body)),
        Some("sha256=00".to_string()),
    ] {
        let r = i.handle(Request {
            head: head("POST", "/github", sig.as_deref(), Some("pull_request")),
            body: body.clone(),
        });
        assert_eq!(r.status, 401);
    }
    // a good signature over a different body is wrong too
    let r = i.handle(Request {
        head: head(
            "POST",
            "/github",
            Some(&signed(b"topsecret", b"{}")),
            Some("pull_request"),
        ),
        body: body.clone(),
    });
    assert_eq!(r.status, 401);
    assert!(q.drain().0.is_empty());
}

#[test]
fn a_signed_ping_or_unwanted_event_is_200_and_nothing_is_queued() {
    let q = Queue::new(8);
    let mut i = intake(&q);
    for (ev, f) in [
        ("ping", "ping"),
        ("pull_request", "pull_request_labeled"),
        ("issues", "ping"),
    ] {
        let body = fixture(f);
        let r = i.handle(Request {
            head: head(
                "POST",
                "/github",
                Some(&signed(b"topsecret", &body)),
                Some(ev),
            ),
            body,
        });
        assert_eq!(r.status, 200, "{ev}");
    }
    assert!(q.drain().0.is_empty());
}

#[test]
fn a_signed_but_malformed_body_or_missing_event_header_is_400_and_nothing_is_queued() {
    let q = Queue::new(8);
    let mut i = intake(&q);
    let body = b"{not json".to_vec();
    let r = i.handle(Request {
        head: head(
            "POST",
            "/github",
            Some(&signed(b"topsecret", &body)),
            Some("pull_request"),
        ),
        body,
    });
    assert_eq!(r.status, 400);
    let body = fixture("pull_request_opened");
    let r = i.handle(Request {
        head: head("POST", "/github", Some(&signed(b"topsecret", &body)), None),
        body,
    });
    assert_eq!(r.status, 400);
    assert!(q.drain().0.is_empty());
}

#[test]
fn the_intake_tracks_heads_across_requests() {
    let q = Queue::new(8);
    let mut i = intake(&q);
    for (ev, f) in [
        ("pull_request", "pull_request_synchronize"),
        ("check_suite", "check_suite_cancelled_no_head"),
    ] {
        let body = fixture(f);
        let r = i.handle(Request {
            head: head(
                "POST",
                "/github",
                Some(&signed(b"topsecret", &body)),
                Some(ev),
            ),
            body,
        });
        assert_eq!(r.status, 200);
    }
    let (got, _) = q.drain();
    assert_eq!(got[1].meta()["conclusion"], "superseded");
}

#[test]
fn a_response_is_a_response_type() {
    // the handler's answers carry fixed text only
    let q = Queue::new(1);
    let mut i = intake(&q);
    let r: Response = i.handle(Request {
        head: head("POST", "/github", None, None),
        body: Vec::new(),
    });
    assert_eq!(r.body, "unauthorized");
}

// ---- delivery id de-duplication (#1089) ----

fn delivered(i: &mut Intake<'_>, id: Option<&str>, body: Vec<u8>, event: &str) -> Response {
    let mut h = head(
        "POST",
        "/github",
        Some(&signed(b"topsecret", &body)),
        Some(event),
    );
    if let Some(id) = id {
        h.headers
            .push(("x-github-delivery".to_string(), id.to_string()));
    }
    i.handle(Request { head: h, body })
}

/// The fixture with `n` trailing spaces: still the same JSON, a different signed body.
fn variant(name: &str, n: usize) -> Vec<u8> {
    let mut b = fixture(name);
    b.extend(std::iter::repeat_n(b' ', n));
    b
}

#[test]
fn a_redelivered_delivery_id_is_answered_200_and_queued_once() {
    let q = Queue::new(8);
    let mut i = intake(&q);
    let id = "72d3162e-cc78-11e3-81ab-4c9367dc0958";
    let a = delivered(
        &mut i,
        Some(id),
        fixture("pull_request_opened"),
        "pull_request",
    );
    let b = delivered(
        &mut i,
        Some(id),
        fixture("pull_request_opened"),
        "pull_request",
    );
    assert_eq!((a.status, b.status), (200, 200));
    assert_eq!(b.body, "duplicate");
    let (got, _) = q.drain();
    assert_eq!(got.len(), 1);
}

#[test]
fn different_delivery_ids_and_missing_ids_are_not_deduplicated() {
    let q = Queue::new(8);
    let mut i = intake(&q);
    for (n, id) in [Some("aaaa-1"), Some("aaaa-2"), None, None]
        .into_iter()
        .enumerate()
    {
        delivered(
            &mut i,
            id,
            variant("pull_request_opened", n),
            "pull_request",
        );
    }
    assert_eq!(q.drain().0.len(), 4);
}

#[test]
fn an_unsigned_repeat_does_not_poison_the_delivery_cache() {
    let q = Queue::new(8);
    let mut i = intake(&q);
    let body = fixture("pull_request_opened");
    let mut h = head("POST", "/github", Some("sha256=00"), Some("pull_request"));
    h.headers
        .push(("x-github-delivery".to_string(), "id-1".to_string()));
    let r = i.handle(Request { head: h, body });
    assert_eq!(r.status, 401);
    let ok = delivered(
        &mut i,
        Some("id-1"),
        fixture("pull_request_opened"),
        "pull_request",
    );
    assert_eq!(ok.body, "ok");
    assert_eq!(q.drain().0.len(), 1);
}

#[test]
fn a_failed_delivery_can_be_redelivered() {
    let q = Queue::new(8);
    let mut i = intake(&q);
    let bad = delivered(&mut i, Some("id-9"), b"not json".to_vec(), "pull_request");
    assert_eq!(bad.status, 400);
    let ok = delivered(
        &mut i,
        Some("id-9"),
        fixture("pull_request_opened"),
        "pull_request",
    );
    assert_eq!(ok.body, "ok");
}

#[test]
fn the_delivery_cache_is_bounded_and_forgets_the_oldest() {
    let q = Queue::new(8);
    let mut i = intake(&q);
    delivered(
        &mut i,
        Some("first"),
        fixture("pull_request_opened"),
        "pull_request",
    );
    for n in 0..DELIVERY_IDS {
        delivered(
            &mut i,
            Some(&format!("id-{n}")),
            variant("pull_request_opened", n + 1),
            "pull_request",
        );
    }
    q.drain();
    let again = delivered(
        &mut i,
        Some("first"),
        fixture("pull_request_opened"),
        "pull_request",
    );
    assert_eq!(again.body, "ok");
    let recent = delivered(
        &mut i,
        Some(&format!("id-{}", DELIVERY_IDS - 1)),
        variant("pull_request_opened", DELIVERY_IDS),
        "pull_request",
    );
    assert_eq!(recent.body, "duplicate");
}

#[test]
fn an_oversized_or_odd_delivery_id_is_ignored_not_cached() {
    let q = Queue::new(8);
    let mut i = intake(&q);
    let long = "a".repeat(200);
    for (k, id) in [long.as_str(), "has space!"].into_iter().enumerate() {
        for m in 0..2 {
            delivered(
                &mut i,
                Some(id),
                variant("pull_request_opened", k * 2 + m),
                "pull_request",
            );
        }
    }
    assert_eq!(q.drain().0.len(), 4);
}

#[test]
fn a_replayed_signed_body_with_a_fresh_delivery_id_is_dropped() {
    let q = Queue::new(8);
    let mut i = intake(&q);
    let a = delivered(
        &mut i,
        Some("id-a"),
        fixture("pull_request_opened"),
        "pull_request",
    );
    let b = delivered(
        &mut i,
        Some("id-b"),
        fixture("pull_request_opened"),
        "pull_request",
    );
    let c = delivered(&mut i, None, fixture("pull_request_opened"), "pull_request");
    assert_eq!((a.body, b.body, c.body), ("ok", "duplicate", "duplicate"));
    assert_eq!(q.drain().0.len(), 1);
}

#[test]
fn the_same_body_under_another_event_name_is_not_a_replay() {
    let q = Queue::new(8);
    let mut i = intake(&q);
    delivered(
        &mut i,
        Some("a"),
        fixture("pull_request_opened"),
        "pull_request",
    );
    let other = delivered(&mut i, Some("b"), fixture("pull_request_opened"), "ping");
    assert_eq!(other.body, "ignored");
}

#[test]
fn an_unsigned_body_does_not_poison_the_content_cache_and_a_failed_one_can_retry() {
    let q = Queue::new(8);
    let mut i = intake(&q);
    let mut h = head("POST", "/github", Some("sha256=00"), Some("pull_request"));
    h.headers
        .push(("x-github-delivery".to_string(), "x".to_string()));
    let r = i.handle(Request {
        head: h,
        body: fixture("pull_request_opened"),
    });
    assert_eq!(r.status, 401);
    let ok = delivered(
        &mut i,
        Some("y"),
        fixture("pull_request_opened"),
        "pull_request",
    );
    assert_eq!(ok.body, "ok");
    let bad = delivered(&mut i, Some("z"), b"{".to_vec(), "pull_request");
    let bad2 = delivered(&mut i, Some("w"), b"{".to_vec(), "pull_request");
    assert_eq!((bad.status, bad2.status), (400, 400));
}

#[test]
fn the_content_cache_is_bounded_and_forgets_the_oldest() {
    let q = Queue::new(8);
    let mut i = intake(&q);
    delivered(
        &mut i,
        None,
        variant("pull_request_opened", 0),
        "pull_request",
    );
    for n in 1..=DELIVERY_IDS {
        delivered(
            &mut i,
            None,
            variant("pull_request_opened", n),
            "pull_request",
        );
    }
    q.drain();
    let first = delivered(
        &mut i,
        None,
        variant("pull_request_opened", 0),
        "pull_request",
    );
    assert_eq!(first.body, "ok");
    let last = delivered(
        &mut i,
        None,
        variant("pull_request_opened", DELIVERY_IDS),
        "pull_request",
    );
    assert_eq!(last.body, "duplicate");
}
