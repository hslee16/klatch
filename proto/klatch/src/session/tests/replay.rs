use super::wire::DownStore;
use super::*;

#[test]
fn subscribe_replays_history_then_caught_up_when_the_input_ends() {
    let g = gateway();
    for i in 0..3 {
        g.post(&bob(), "r", &format!("m{i}")).unwrap();
    }
    let f = run(
        &g,
        &alice(),
        &[req("s", "subscribe", json!({"room":"r","from_seq":0}))],
    );
    let (ok, result, _) = response(&f, "s");
    assert!(ok);
    assert_eq!(result.unwrap()["head"], 3);
    assert_eq!(seqs(&f), vec![1, 2, 3]);
    let tail = f.last().unwrap();
    assert!(
        matches!(tail, ServerFrame::CaughtUp { room, seq: 3, .. } if room == "r"),
        "{tail:?}"
    );
    let resp_at = f
        .iter()
        .position(|x| matches!(x, ServerFrame::Response { .. }))
        .unwrap();
    let first_msg = f
        .iter()
        .position(|x| matches!(x, ServerFrame::Message { .. }))
        .unwrap();
    assert!(resp_at < first_msg, "the response comes before the replay");
}

#[test]
fn subscribe_from_a_seq_skips_what_the_client_already_has() {
    let g = gateway();
    for i in 0..4 {
        g.post(&bob(), "r", &format!("m{i}")).unwrap();
    }
    let f = run(
        &g,
        &alice(),
        &[req("s", "subscribe", json!({"room":"r","from_seq":2}))],
    );
    assert_eq!(seqs(&f), vec![3, 4]);
}

#[test]
fn subscribing_to_an_empty_room_reports_caught_up_at_zero() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[req("s", "subscribe", json!({"room":"r","from_seq":0}))],
    );
    assert!(matches!(
        f.last().unwrap(),
        ServerFrame::CaughtUp { seq: 0, .. }
    ));
}

#[test]
fn the_stream_includes_the_subscribers_own_messages() {
    let g = gateway();
    g.post(&alice(), "r", "mine").unwrap();
    g.post(&bob(), "r", "theirs").unwrap();
    let f = run(
        &g,
        &alice(),
        &[req("s", "subscribe", json!({"room":"r","from_seq":0}))],
    );
    assert_eq!(seqs(&f), vec![1, 2]);
}

#[test]
fn a_bad_room_or_missing_from_seq_is_refused_at_subscribe_time() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[
            req("1", "subscribe", json!({"room":"bad room","from_seq":0})),
            req("2", "subscribe", json!({"room":"r"})),
            req("3", "subscribe", json!({"room":"r","from_seq":-4})),
        ],
    );
    assert_eq!(response(&f, "1").2.unwrap().0, "invalid");
    assert_eq!(response(&f, "2").2.unwrap().0, "invalid_params");
    assert_eq!(response(&f, "3").2.unwrap().0, "invalid_params");
    assert!(messages(&f).is_empty());
}

struct DenyStream;
impl Guard for DenyStream {
    fn check(&self, _who: &Principal, op: &Operation) -> Result<()> {
        if matches!(op, Operation::Subscribe { .. }) {
            return Err(Error::Refused {
                rule: "test.no_stream".into(),
                reason: "streaming is not allowed here".into(),
            });
        }
        Ok(())
    }
}

#[test]
fn a_guard_can_refuse_a_subscription_and_nothing_streams() {
    let g = Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        FixedClock(1),
        DenyStream,
    );
    g.post(&bob(), "r", "secret").unwrap();
    let f = run(
        &g,
        &alice(),
        &[req("s", "subscribe", json!({"room":"r","from_seq":0}))],
    );
    assert!(f.iter().any(|x| matches!(x, ServerFrame::Refusal { request: Some(r), rule, .. } if r == "s" && rule == "test.no_stream")));
    assert_eq!(response(&f, "s").2.unwrap().0, "refused");
    assert!(messages(&f).is_empty());
}

#[test]
fn a_store_failure_at_subscribe_is_a_failed_response() {
    let g = Gateway::new(DownStore, FixedClock(1), AllowAll);
    let f = run(
        &g,
        &alice(),
        &[req("s", "subscribe", json!({"room":"r","from_seq":0}))],
    );
    assert_eq!(response(&f, "s").2.unwrap().0, "backend");
}

#[test]
fn unsubscribing_is_idempotent_and_answers_ok() {
    let g = gateway();
    let f = run(
        &g,
        &alice(),
        &[req("u", "unsubscribe", json!({"room":"r"}))],
    );
    assert!(response(&f, "u").0);
}

#[test]
fn an_ack_frame_advances_the_members_read_cursor() {
    let g = gateway();
    g.join(&alice(), "r").unwrap();
    for i in 0..3 {
        g.post(&bob(), "r", &format!("m{i}")).unwrap();
    }
    run(
        &g,
        &alice(),
        &[json!({"v":1,"type":"ack","room":"r","seq":2})],
    );
    let left = g.read(&alice(), "r", false).unwrap();
    assert_eq!(left.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![3]);
}

// ---- live sessions: a thread, a channel-backed input, a shared output, a file database ----
