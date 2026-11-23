//! The conformance scenario of the wire tests (#108, #112): one list of frames a member sends, the normalisation that drops
//! what differs between two runs (times, random ids), and the comparison of two runs (responses in order, the stream's other
//! frames as a set). `tls_wire.rs` runs it over SSE on loopback and on TLS; `ws_wire.rs` runs it over WebSocket as well.
#![allow(dead_code)]

use super::wire::{Net, Sse};
use serde_json::{Value, json};
use std::time::Duration;

pub fn scenario(member: &str) -> Vec<Value> {
    let req = |id: &str, method: &str, params: Value| json!({"v":1,"type":"request","id":id,"method":method,"params":params});
    let mut s = vec![req("j", "join", json!({"room": "conf"}))];
    for i in 0..3 {
        s.push(req(
            &format!("p{i}"),
            "post",
            json!({"room": "conf", "body": format!("{member}-{i}")}),
        ));
    }
    s.push(req(
        "post-handoff",
        "post",
        json!({"room": "conf", "body": "h", "kind": "review", "payload": "{\"n\":1}"}),
    ));
    s.push(req("head", "history", json!({"room": "conf", "last": 10})));
    s.push(req(
        "sub",
        "subscribe",
        json!({"room": "conf", "from_seq": 0}),
    ));
    s.push(json!({"v":1,"type":"ack","room":"conf","seq":2}));
    s.push(req("read", "read", json!({"room": "conf"})));
    s.push(req(
        "who",
        "claim",
        json!({"room": "conf", "path": format!("{member}.rs")}),
    ));
    s.push(req("bad", "no_such_method", json!({})));
    s
}

/// Drop what differs between two runs of the same thing (times, random ids); keep every sequence number, sender,
/// attribution, body and error code.
pub fn normal(mut v: Value, top: bool) -> Value {
    match &mut v {
        Value::Object(m) => {
            for k in ["ts", "last_seen", "expires"] {
                m.remove(k);
            }
            if !top || m.get("type").is_some_and(|t| t != "response") {
                m.remove("id");
            }
            for (_, x) in m.iter_mut() {
                *x = normal(x.take(), false);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| *x = normal(x.take(), false)),
        _ => {}
    }
    v
}

/// Responses in request order; the stream's other frames as a sorted set (their moments depend on timing).
pub fn split(frames: &[Value]) -> (Vec<Value>, Vec<String>) {
    let responses = frames
        .iter()
        .filter(|f| f["type"] == "response")
        .cloned()
        .collect();
    let mut rest: Vec<String> = frames
        .iter()
        .filter(|f| f["type"] != "response")
        .map(|f| f.to_string())
        .collect();
    rest.sort();
    (responses, rest)
}

/// The scenario of one member through `net`: the frames after the hello, in the order they arrived.
pub fn run_scenario(net: &Net, name: &str, token: &str) -> Vec<Value> {
    let mut c = Sse::open(net, name, token);
    let hello = c.recv().unwrap();
    assert_eq!(
        (hello["member"].as_str(), hello["attribution"].as_str()),
        (Some(name), Some("verified"))
    );
    for f in scenario(name) {
        assert_eq!(c.post_frame(&f), 202);
    }
    let mut frames = Vec::new();
    loop {
        let f = c.recv().expect("a frame");
        let last = f["type"] == "response" && f["id"] == "bad";
        frames.push(f);
        if last {
            break;
        }
    }
    c.r.get_ref().set_read_timeout(Duration::from_millis(700));
    while let Some(f) = c.recv() {
        frames.push(f);
    }
    frames
        .into_iter()
        .filter(|f| f["type"] != "heartbeat")
        .map(|f| normal(f, true))
        .collect()
}
