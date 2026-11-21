use super::{Session, SessionConfig, serve};
use crate::domain::{Claim, Error, MemberInfo, Message, Posted, Principal, Result};
use crate::gateway::{AllowAll, FixedClock, Gateway, Guard, Operation};
use crate::protocol::{Attribution, ServerFrame, decode_server_line};
use crate::store::{
    AskStore, ClaimRequest, ClaimStore, CredentialStore, MemberStore, MessageStore, NewMessage,
    PolicyStore, ReviewStore, RoomStore, Seat, Seq, SqliteStore, Store,
};
use serde_json::{Value, json};
use std::io::Cursor;

fn gateway() -> Gateway<SqliteStore> {
    Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        FixedClock(1_000),
        AllowAll,
    )
}

/// Run a session over the given input lines and decode every output line through the real decoder.
fn run<S: Store>(g: &Gateway<S>, who: &Principal, lines: &[Value]) -> Vec<ServerFrame> {
    let input: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let mut out = Vec::new();
    serve(g, who, Cursor::new(input), &mut out).unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| decode_server_line(l).unwrap_or_else(|e| panic!("bad output {l}: {e:?}")))
        .collect()
}

fn req(id: &str, method: &str, params: Value) -> Value {
    json!({"v":1,"type":"request","id":id,"method":method,"params":params})
}

fn alice() -> Principal {
    Principal::unattributed("alice")
}
fn bob() -> Principal {
    Principal::unattributed("bob")
}

fn response(frames: &[ServerFrame], id: &str) -> (bool, Option<Value>, Option<(String, String)>) {
    for f in frames {
        if let ServerFrame::Response {
            id: rid,
            ok,
            result,
            error,
            ..
        } = f
            && rid == id
        {
            return (
                *ok,
                result.clone(),
                error.as_ref().map(|e| (e.code.clone(), e.message.clone())),
            );
        }
    }
    panic!("no response for {id}: {frames:?}");
}

/// Send one request as alice and assert the response is a failed one with the `invalid` code.
fn assert_invalid_request(method: &str, params: Value) {
    let f = run(&gateway(), &alice(), &[req("s", method, params)]);
    let (ok, _, err) = response(&f, "s");
    assert!(!ok);
    assert_eq!(err.unwrap().0, "invalid");
}

pub(super) fn frames_of(out: &[u8]) -> Vec<ServerFrame> {
    String::from_utf8(out.to_vec())
        .unwrap()
        .lines()
        .map(|l| decode_server_line(l).unwrap_or_else(|e| panic!("bad output {l}: {e:?}")))
        .collect()
}

fn messages(f: &[ServerFrame]) -> Vec<(String, u64, String)> {
    f.iter()
        .filter_map(|x| match x {
            ServerFrame::Message {
                room, seq, body, ..
            } => Some((room.clone(), *seq, body.clone())),
            _ => None,
        })
        .collect()
}

fn seqs(f: &[ServerFrame]) -> Vec<u64> {
    messages(f).into_iter().map(|m| m.1).collect()
}

mod attributes;
mod crossings_wire;
mod dm;
mod facilitator_wire;
mod fixed_member;
mod grant;
mod guarded;
mod live;
mod mute;
mod my_role_wire;
mod own_record_wire;
mod recheck;
mod replay;
mod stuck;
mod wire;
