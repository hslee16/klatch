//! klatch-session: one gateway session over any duplex byte stream, speaking `PROTOCOL.md`.
//! Identity comes from the `Principal` the transport supplies, never from a request.

use klatch_core::{Attribution as Who, Claim, Error, MemberInfo, Message, Posted, Principal};
use klatch_gateway::Gateway;
use klatch_protocol::{
    Attribution, ClientFrame, DecodeError, ErrorBody, FrameReader, PROTOCOL_VERSION, ServerFrame,
    encode_line,
};
use klatch_store::Store;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

/// Time to live of a claim when the request does not give one, in seconds.
const DEFAULT_TTL: i64 = 600;

/// Until the label model exists every caller has the lowest clearance; this is the safe direction.
const DEFAULT_CLEARANCE: &str = "public";

/// Run a session: send `hello`, then answer every request with exactly one `response` until the input ends.
///
/// `subscribe` and live streaming arrive with the streaming ticket; until then `subscribe` answers
/// `not_implemented`, and acknowledgements and unknown frames are accepted silently.
pub fn serve<S: Store, R: BufRead, W: Write>(
    gateway: &Gateway<S>,
    who: &Principal,
    input: R,
    mut output: W,
) -> io::Result<()> {
    send(
        &mut output,
        &ServerFrame::Hello {
            v: PROTOCOL_VERSION,
            member: who.member.clone(),
            attribution: attribution(who.attribution),
            clearance: DEFAULT_CLEARANCE.into(),
        },
    )?;
    let mut reader = FrameReader::new(input);
    while let Some(item) = reader.next_client() {
        match item {
            Ok(ClientFrame::Request {
                id, method, params, ..
            }) => {
                for frame in handle(gateway, who, id, &method, params) {
                    send(&mut output, &frame)?;
                }
            }
            Ok(ClientFrame::Ack { .. } | ClientFrame::Unknown) => {}
            Err(DecodeError::Io(e)) => return Err(io::Error::other(e)),
            Err(e) => send(&mut output, &decode_refusal(&e))?,
        }
    }
    Ok(())
}

fn send<W: Write>(out: &mut W, frame: &ServerFrame) -> io::Result<()> {
    out.write_all(encode_line(frame).as_bytes())?;
    out.flush()
}

fn attribution(a: Who) -> Attribution {
    match a {
        Who::Verified => Attribution::Verified,
        Who::Unattributed => Attribution::Unattributed,
    }
}

/// A refusal for a frame that could not be read. Says what was wrong, never what the frame contained.
fn decode_refusal(e: &DecodeError) -> ServerFrame {
    let (rule, reason) = match e {
        DecodeError::MissingVersion => (
            "protocol.version",
            "the frame has no protocol version".to_string(),
        ),
        DecodeError::UnsupportedVersion(n) => (
            "protocol.version",
            format!(
                "protocol version {n} is not supported; this gateway speaks {PROTOCOL_VERSION}"
            ),
        ),
        DecodeError::TooLong(_) => (
            "protocol.decode",
            "the line is longer than the limit".to_string(),
        ),
        DecodeError::Empty | DecodeError::Json(_) | DecodeError::Shape(_) | DecodeError::Io(_) => {
            ("protocol.decode", "the frame could not be read".to_string())
        }
    };
    ServerFrame::Refusal {
        v: PROTOCOL_VERSION,
        request: None,
        rule: rule.into(),
        reason,
    }
}

fn ok(id: String, result: Value) -> ServerFrame {
    ServerFrame::Response {
        v: PROTOCOL_VERSION,
        id,
        ok: true,
        result: Some(result),
        error: None,
    }
}

fn fail_with(id: String, code: &str, message: String) -> ServerFrame {
    ServerFrame::Response {
        v: PROTOCOL_VERSION,
        id,
        ok: false,
        result: None,
        error: Some(ErrorBody {
            code: code.into(),
            message,
        }),
    }
}

/// The frames that answer a failed request. A policy refusal also names its rule in a `refusal` frame.
fn failure(id: String, e: &Error) -> Vec<ServerFrame> {
    let (code, message) = match e {
        Error::Invalid(m) => ("invalid", m.clone()),
        Error::Conflict { .. } => ("conflict", e.to_string()),
        Error::Refused { reason, .. } => ("refused", reason.clone()),
        Error::NotFound(m) => ("not_found", m.clone()),
        Error::Busy(m) => ("busy", m.clone()),
        Error::Backend(m) => ("backend", m.clone()),
    };
    let mut frames = Vec::new();
    if let Error::Refused { rule, reason } = e {
        frames.push(ServerFrame::Refusal {
            v: PROTOCOL_VERSION,
            request: Some(id.clone()),
            rule: rule.clone(),
            reason: reason.clone(),
        });
    }
    frames.push(fail_with(id, code, message));
    frames
}

#[derive(Deserialize)]
struct RoomParams {
    room: String,
}
#[derive(Deserialize)]
struct PostParams {
    room: String,
    body: String,
}
#[derive(Deserialize)]
struct ReadParams {
    room: String,
    #[serde(default)]
    peek: bool,
}
#[derive(Deserialize)]
struct ClaimParams {
    room: String,
    path: String,
    ttl: Option<i64>,
    #[serde(default)]
    reason: String,
}
#[derive(Deserialize)]
struct PathParams {
    room: String,
    path: String,
}

fn params<T: DeserializeOwned>(id: &str, value: Value) -> Result<T, Vec<ServerFrame>> {
    serde_json::from_value(value).map_err(|_| {
        vec![fail_with(
            id.to_string(),
            "invalid_params",
            "the request parameters are missing or have the wrong shape".into(),
        )]
    })
}

fn message_json(m: &Message) -> Value {
    // Stored messages do not yet record whether their sender was proven or what label they carry, so they are
    // reported as unattributed and public, which is the cautious reading (R23, DESIGN D3).
    json!({
        "id": m.id, "seq": m.seq, "sender": m.sender, "attribution": "unattributed",
        "label": "public", "body": m.body, "ts": m.ts
    })
}

fn handle<S: Store>(
    g: &Gateway<S>,
    who: &Principal,
    id: String,
    method: &str,
    value: Value,
) -> Vec<ServerFrame> {
    macro_rules! parse {
        ($t:ty) => {
            match params::<$t>(&id, value) {
                Ok(p) => p,
                Err(frames) => return frames,
            }
        };
    }
    macro_rules! answer {
        ($result:expr, $shape:expr) => {
            match $result {
                Ok(v) => vec![ok(id, $shape(v))],
                Err(e) => failure(id, &e),
            }
        };
    }
    match method {
        "post" => {
            let p = parse!(PostParams);
            answer!(g.post(who, &p.room, &p.body), |x: Posted| json!({
                "seq": x.seq, "id": x.id
            }))
        }
        "read" => {
            let p = parse!(ReadParams);
            answer!(g.read(who, &p.room, !p.peek), |ms: Vec<Message>| json!({
                "messages": ms.iter().map(message_json).collect::<Vec<_>>()
            }))
        }
        "who" => {
            let p = parse!(RoomParams);
            answer!(g.who(who, &p.room), |ms: Vec<MemberInfo>| json!({
                "members": ms.iter()
                    .map(|m| json!({"name": m.name, "last_seen": m.last_seen}))
                    .collect::<Vec<_>>()
            }))
        }
        "claim" => {
            let p = parse!(ClaimParams);
            let ttl = p.ttl.unwrap_or(DEFAULT_TTL);
            answer!(g.claim(who, &p.room, &p.path, ttl, &p.reason), |()| json!(
                {}
            ))
        }
        "release" => {
            let p = parse!(PathParams);
            answer!(g.release(who, &p.room, &p.path), |()| json!({}))
        }
        "claims" => {
            let p = parse!(RoomParams);
            answer!(g.claims(who, &p.room), |cs: Vec<Claim>| json!({
                "claims": cs.iter()
                    .map(|c| json!({
                        "path": c.path, "owner": c.owner, "reason": c.reason, "expires": c.expires
                    }))
                    .collect::<Vec<_>>()
            }))
        }
        "subscribe" | "unsubscribe" => vec![fail_with(
            id,
            "not_implemented",
            "streaming arrives with the subscribe ticket".into(),
        )],
        _ => vec![fail_with(
            id,
            "unknown_method",
            format!("unknown method {method}"),
        )],
    }
}

#[cfg(test)]
mod tests;
