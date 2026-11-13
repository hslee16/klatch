//! klatch-session: one gateway session over any duplex byte stream, speaking `PROTOCOL.md`.
//! Identity comes from the `Principal` the transport supplies, never from a request.

use klatch_core::{Attribution as Who, Claim, Error, MemberInfo, Message, Posted, Principal};
use klatch_gateway::{Gateway, Subscription};
use klatch_protocol::{
    Attribution, ClientFrame, DecodeError, ErrorBody, FrameReader, PROTOCOL_VERSION, ServerFrame,
    encode_line,
};
use klatch_store::Store;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
use std::sync::mpsc::{RecvTimeoutError, TryRecvError, channel};
use std::time::{Duration, Instant};

/// Time to live of a claim when the request does not give one, in seconds.
const DEFAULT_TTL: i64 = 600;

/// Until the label model exists every caller has the lowest clearance; this is the safe direction.
const DEFAULT_CLEARANCE: &str = "public";

/// Timing and flow-control settings of a session.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// How often the store is polled for new messages in subscribed rooms.
    pub poll: Duration,
    /// How long the stream may be idle before a `heartbeat` is sent.
    pub heartbeat: Duration,
    /// Messages a subscriber may have outstanding (sent, not acknowledged) per room.
    pub window: usize,
    /// The most messages sent for one room in one poll.
    pub batch: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            poll: Duration::from_millis(100),
            heartbeat: Duration::from_secs(15),
            window: klatch_gateway::DEFAULT_WINDOW,
            batch: 64,
        }
    }
}

/// Run a session with the default settings; see `serve_with`.
pub fn serve<S: Store, R: BufRead + Send + 'static, W: Write>(
    gateway: &Gateway<S>,
    who: &Principal,
    input: R,
    output: W,
) -> io::Result<()> {
    serve_with(gateway, who, input, output, &SessionConfig::default())
}

/// Run a session: send `hello`, answer every request with exactly one `response`, and push stream frames
/// for subscribed rooms, until the input ends.
///
/// Input is read on a helper thread while this thread handles requests, polls the store for new messages,
/// pushes them and sends heartbeats when idle. When the input ends the session delivers whatever replay is
/// immediately available (within each subscription's window) and returns.
pub fn serve_with<S: Store, R: BufRead + Send + 'static, W: Write>(
    gateway: &Gateway<S>,
    who: &Principal,
    input: R,
    output: W,
    config: &SessionConfig,
) -> io::Result<()> {
    let mut out = Out {
        w: output,
        last: Instant::now(),
    };
    out.send(&ServerFrame::Hello {
        v: PROTOCOL_VERSION,
        member: who.member.clone(),
        attribution: attribution(who.attribution),
        clearance: DEFAULT_CLEARANCE.into(),
    })?;
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut reader = FrameReader::new(input);
        while let Some(item) = reader.next_client() {
            if tx.send(item).is_err() {
                break;
            }
        }
    });
    let mut subs: Vec<Subscription> = Vec::new();
    let mut open = true;
    while open {
        let mut items = Vec::new();
        match rx.recv_timeout(config.poll) {
            Ok(item) => items.push(item),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => open = false,
        }
        while open {
            match rx.try_recv() {
                Ok(item) => items.push(item),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => open = false,
            }
        }
        for item in items {
            match item {
                Ok(ClientFrame::Request {
                    id, method, params, ..
                }) => {
                    let frames = request(gateway, who, config, &mut subs, id, &method, params);
                    out.send_all(&frames)?;
                }
                Ok(ClientFrame::Ack { room, seq, .. }) => {
                    let frames = acknowledge(gateway, who, &mut subs, &room, seq);
                    out.send_all(&frames)?;
                }
                Ok(ClientFrame::Unknown) => {}
                Err(DecodeError::Io(e)) => return Err(io::Error::other(e)),
                Err(e) => out.send(&decode_refusal(&e))?,
            }
        }
        pump(gateway, who, config, &mut subs, &mut out)?;
        if !open {
            // The input is closed: deliver what replay is immediately available, then end.
            while pump(gateway, who, config, &mut subs, &mut out)? > 0 {}
        } else if out.last.elapsed() >= config.heartbeat {
            out.send(&ServerFrame::Heartbeat {
                v: PROTOCOL_VERSION,
                ts: gateway.now(),
            })?;
        }
    }
    Ok(())
}

/// Write the event stream of one room to `output` with no request channel: `hello`, the history after
/// `from_seq`, `caught_up`, and, with `follow`, live messages and heartbeats until the output closes.
///
/// This is the daemon-free producer behind `klatch stream`. A pipe reader cannot acknowledge, so there is
/// no flow-control window. The room is checked before anything is written, so a refused or invalid room
/// leaves the output empty and returns an error.
pub fn stream<S: Store, W: Write>(
    gateway: &Gateway<S>,
    who: &Principal,
    room: &str,
    from_seq: u64,
    follow: bool,
    output: W,
    config: &SessionConfig,
) -> io::Result<()> {
    gateway.head(who, room).map_err(io::Error::other)?;
    let mut out = Out {
        w: output,
        last: Instant::now(),
    };
    out.send(&ServerFrame::Hello {
        v: PROTOCOL_VERSION,
        member: who.member.clone(),
        attribution: attribution(who.attribution),
        clearance: DEFAULT_CLEARANCE.into(),
    })?;
    let from = i64::try_from(from_seq).unwrap_or(i64::MAX);
    let mut subs = vec![Subscription::new(room, from, usize::MAX / 2)];
    loop {
        pump(gateway, who, config, &mut subs, &mut out)?;
        if subs.is_empty() {
            return Err(io::Error::other("the stream ended after a refusal"));
        }
        if !follow && subs[0].is_caught_up() {
            return Ok(());
        }
        if follow && out.last.elapsed() >= config.heartbeat {
            out.send(&ServerFrame::Heartbeat {
                v: PROTOCOL_VERSION,
                ts: gateway.now(),
            })?;
        }
        std::thread::sleep(config.poll);
    }
}

/// The output stream and when anything was last written to it.
struct Out<W: Write> {
    w: W,
    last: Instant,
}

impl<W: Write> Out<W> {
    fn send(&mut self, frame: &ServerFrame) -> io::Result<()> {
        send(&mut self.w, frame)?;
        self.last = Instant::now();
        Ok(())
    }

    fn send_all(&mut self, frames: &[ServerFrame]) -> io::Result<()> {
        frames.iter().try_for_each(|f| self.send(f))
    }
}

/// A refusal for a stream failure. Transient lock contention (`Busy`) is retried and never reported.
fn stream_refusal(e: &Error) -> Option<ServerFrame> {
    let (rule, reason) = match e {
        Error::Busy(_) => return None,
        Error::Refused { rule, reason } => (rule.clone(), reason.clone()),
        Error::Backend(m) => ("stream.backend".to_string(), m.clone()),
        Error::Invalid(m) => ("stream.invalid".to_string(), m.clone()),
        Error::NotFound(m) => ("stream.not_found".to_string(), m.clone()),
        Error::Conflict { .. } => ("stream.conflict".to_string(), e.to_string()),
    };
    Some(ServerFrame::Refusal {
        v: PROTOCOL_VERSION,
        request: None,
        rule,
        reason,
    })
}

/// Handle an acknowledgement: move the subscription's window and the member's read cursor.
fn acknowledge<S: Store>(
    gateway: &Gateway<S>,
    who: &Principal,
    subs: &mut [Subscription],
    room: &str,
    seq: u64,
) -> Vec<ServerFrame> {
    let seq = i64::try_from(seq).unwrap_or(i64::MAX);
    if let Some(sub) = subs.iter_mut().find(|s| s.room() == room) {
        sub.ack(seq);
    }
    match gateway.ack(who, room, seq) {
        Ok(()) => Vec::new(),
        Err(e) => stream_refusal(&e).into_iter().collect(),
    }
}

/// Poll every subscription once (draining a room while batches come back full) and send what they yield.
/// Returns how many frames were sent. A subscription that fails is dropped after a refusal.
fn pump<S: Store, W: Write>(
    gateway: &Gateway<S>,
    who: &Principal,
    config: &SessionConfig,
    subs: &mut Vec<Subscription>,
    out: &mut Out<W>,
) -> io::Result<usize> {
    let mut sent = 0;
    let mut dropped: Vec<String> = Vec::new();
    for sub in subs.iter_mut() {
        loop {
            match sub.poll(gateway, who, config.batch) {
                Ok(batch) => {
                    let full = batch.messages.len() >= config.batch;
                    for m in &batch.messages {
                        out.send(&message_frame(sub.room(), m))?;
                        sent += 1;
                    }
                    if let Some(seq) = batch.caught_up {
                        out.send(&ServerFrame::CaughtUp {
                            v: PROTOCOL_VERSION,
                            room: sub.room().to_string(),
                            seq: u64::try_from(seq).unwrap_or(0),
                        })?;
                        sent += 1;
                    }
                    if !full {
                        break;
                    }
                }
                Err(e) => {
                    if let Some(frame) = stream_refusal(&e) {
                        out.send(&frame)?;
                        sent += 1;
                        dropped.push(sub.room().to_string());
                    }
                    break;
                }
            }
        }
    }
    subs.retain(|s| !dropped.iter().any(|r| r == s.room()));
    Ok(sent)
}

fn message_frame(room: &str, m: &Message) -> ServerFrame {
    ServerFrame::Message {
        v: PROTOCOL_VERSION,
        room: room.to_string(),
        seq: u64::try_from(m.seq).unwrap_or(0),
        id: m.id.clone(),
        sender: m.sender.clone(),
        attribution: attribution(m.attribution),
        label: m.label.clone(),
        body: m.body.clone(),
        ts: m.ts,
    }
}

#[derive(Deserialize)]
struct SubscribeParams {
    room: String,
    from_seq: u64,
}

/// Dispatch one request. `subscribe` and `unsubscribe` change the session's subscriptions; every other
/// method is a plain call through the gateway.
fn request<S: Store>(
    gateway: &Gateway<S>,
    who: &Principal,
    config: &SessionConfig,
    subs: &mut Vec<Subscription>,
    id: String,
    method: &str,
    value: Value,
) -> Vec<ServerFrame> {
    match method {
        "subscribe" => {
            let p = match params::<SubscribeParams>(&id, value) {
                Ok(p) => p,
                Err(frames) => return frames,
            };
            match gateway.head(who, &p.room) {
                Ok(head) => {
                    subs.retain(|s| s.room() != p.room);
                    let from = i64::try_from(p.from_seq).unwrap_or(i64::MAX);
                    subs.push(Subscription::new(&p.room, from, config.window));
                    vec![ok(id, json!({ "head": head }))]
                }
                Err(e) => failure(id, &e),
            }
        }
        "unsubscribe" => match params::<RoomParams>(&id, value) {
            Ok(p) => {
                subs.retain(|s| s.room() != p.room);
                vec![ok(id, json!({}))]
            }
            Err(frames) => frames,
        },
        _ => handle(gateway, who, id, method, value),
    }
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
    json!({
        "id": m.id, "seq": m.seq, "sender": m.sender,
        "attribution": match m.attribution {
            Who::Verified => "verified",
            Who::Unattributed => "unattributed",
        },
        "label": m.label, "body": m.body, "ts": m.ts
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
        _ => vec![fail_with(
            id,
            "unknown_method",
            format!("unknown method {method}"),
        )],
    }
}

#[cfg(test)]
mod tests;
