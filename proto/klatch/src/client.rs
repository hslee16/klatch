//! A thin klatch client over the wire protocol (PROTOCOL.md, version 1): it speaks newline-delimited JSON to
//! `klatch serve --stdio` using the same frame types the gateway does. Who you are is decided by the environment the
//! process is started with, never by a request parameter. A frame type or field this version does not know is ignored.

use crate::protocol::{
    Attribution, ClientFrame, PROTOCOL_VERSION, ServerFrame, decode_server_line, encode_line,
};
use serde_json::{Map, Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

/// A failed response. `refusals` are the refusal frames that came with it: a policy refused the call.
#[derive(Debug)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub refusals: Vec<(String, String)>,
}

#[derive(Debug)]
pub enum Error {
    /// The session could not be started, or ended before it answered.
    Session(String),
    /// A frame that is not version 1 of the protocol.
    Protocol(String),
    Api(ApiError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Session(m) | Self::Protocol(m) => f.write_str(m),
            Self::Api(e) => write!(f, "{}: {}", e.code, e.message),
        }
    }
}
impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// A message as delivered, by a stream or by `read`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub seq: u64,
    pub id: String,
    pub sender: String,
    pub attribution: String,
    pub label: String,
    pub body: String,
    pub ts: i64,
    pub kind: Option<String>,
    pub payload: Option<String>,
}

/// What `collect` returns.
#[derive(Debug, Default)]
pub struct Collected {
    pub messages: Vec<Message>,
    pub caught_up: Option<u64>,
}

/// One session with the gateway. Not shared between threads: use one client per thread.
pub struct Client {
    pub member: String,
    pub attribution: String,
    pub clearance: String,
    child: Child,
    stdin: Option<ChildStdin>,
    frames: Receiver<Option<std::result::Result<ServerFrame, String>>>,
    next_id: u64,
    refusals: Vec<(String, String)>,
    pending: Vec<Message>,
    caught_up: Option<u64>,
}

fn attribution_text(a: &Attribution) -> String {
    match a {
        Attribution::Verified => "verified",
        Attribution::Unattributed => "unattributed",
    }
    .into()
}

impl Client {
    /// Start `binary serve --stdio` with exactly `env` as its environment and read its hello.
    pub fn connect(binary: &Path, env: &[(String, String)]) -> Result<Self> {
        let mut child = Command::new(binary)
            .args(["serve", "--stdio"])
            .env_clear()
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Error::Session(format!("could not start {}: {e}", binary.display())))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Session("no output".into()))?;
        let stdin = child.stdin.take();
        let (tx, frames) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                let frame = decode_server_line(&line).map_err(|e| format!("{e:?}"));
                if tx.send(Some(frame)).is_err() {
                    return;
                }
            }
            let _ = tx.send(None);
        });
        let mut c = Self {
            member: String::new(),
            attribution: String::new(),
            clearance: String::new(),
            child,
            stdin,
            frames,
            next_id: 0,
            refusals: Vec::new(),
            pending: Vec::new(),
            caught_up: None,
        };
        match c.frame(Duration::from_secs(10))? {
            Some(ServerFrame::Hello {
                v,
                member,
                attribution,
                clearance,
            }) if v == PROTOCOL_VERSION => {
                c.member = member;
                c.attribution = attribution_text(&attribution);
                c.clearance = clearance;
                Ok(c)
            }
            _ => Err(Error::Protocol("the first frame is hello".into())),
        }
    }

    fn frame(&mut self, timeout: Duration) -> Result<Option<ServerFrame>> {
        match self.frames.recv_timeout(timeout) {
            Ok(Some(Ok(frame))) => Ok(Some(frame)),
            Ok(Some(Err(e))) => Err(Error::Protocol(e)),
            Ok(None) | Err(RecvTimeoutError::Disconnected) => {
                Err(Error::Session("the gateway closed the session".into()))
            }
            Err(RecvTimeoutError::Timeout) => Ok(None),
        }
    }

    fn note(&mut self, frame: ServerFrame) {
        match frame {
            ServerFrame::Message {
                seq,
                id,
                sender,
                attribution,
                label,
                body,
                ts,
                kind,
                payload,
                ..
            } => self.pending.push(Message {
                seq,
                id,
                sender,
                attribution: attribution_text(&attribution),
                label,
                body,
                ts,
                kind,
                payload,
            }),
            ServerFrame::CaughtUp { seq, .. } => self.caught_up = Some(seq),
            ServerFrame::Refusal { rule, reason, .. } => self.refusals.push((rule, reason)),
            _ => {}
        }
    }

    fn send(&mut self, frame: &ClientFrame) -> Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| Error::Session("the session is closed".into()))?;
        stdin
            .write_all(encode_line(frame).as_bytes())
            .and_then(|()| stdin.flush())
            .map_err(|e| Error::Session(e.to_string()))
    }

    /// Send a request and return its result, or `Error::Api`. Frames that arrive meanwhile are kept.
    pub fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        let id = format!("c{}", self.next_id);
        self.refusals.clear();
        self.send(&ClientFrame::Request {
            v: PROTOCOL_VERSION,
            id: id.clone(),
            method: method.into(),
            params,
        })?;
        loop {
            match self.frame(Duration::from_secs(30))? {
                None => return Err(Error::Session("the gateway did not answer".into())),
                Some(ServerFrame::Response {
                    id: rid,
                    ok,
                    result,
                    error,
                    ..
                }) if rid == id => {
                    if ok {
                        return Ok(result.unwrap_or(Value::Null));
                    }
                    let e = error.unwrap_or(crate::protocol::ErrorBody {
                        code: "backend".into(),
                        message: String::new(),
                    });
                    return Err(Error::Api(ApiError {
                        code: e.code,
                        message: e.message,
                        refusals: std::mem::take(&mut self.refusals),
                    }));
                }
                Some(other) => self.note(other),
            }
        }
    }

    pub fn post(
        &mut self,
        room: &str,
        body: &str,
        label: Option<&str>,
        handoff: Option<(&str, Option<&str>)>,
    ) -> Result<(u64, String)> {
        let mut p = Map::new();
        p.insert("room".into(), json!(room));
        p.insert("body".into(), json!(body));
        if let Some(l) = label {
            p.insert("label".into(), json!(l));
        }
        if let Some((kind, payload)) = handoff {
            p.insert("kind".into(), json!(kind));
            if let Some(payload) = payload {
                p.insert("payload".into(), json!(payload));
            }
        }
        let r = self.request("post", Value::Object(p))?;
        Ok((
            r["seq"].as_u64().unwrap_or(0),
            r["id"].as_str().unwrap_or("").to_string(),
        ))
    }

    pub fn read(&mut self, room: &str, peek: bool) -> Result<Vec<Message>> {
        let r = self.request("read", json!({"room": room, "peek": peek}))?;
        Ok(r["messages"]
            .as_array()
            .map(|a| a.iter().map(message_of).collect())
            .unwrap_or_default())
    }

    pub fn claim(
        &mut self,
        room: &str,
        path: &str,
        ttl: Option<i64>,
        reason: Option<&str>,
    ) -> Result<()> {
        let mut p = json!({"room": room, "path": path});
        if let Some(t) = ttl {
            p["ttl"] = json!(t);
        }
        if let Some(r) = reason {
            p["reason"] = json!(r);
        }
        self.request("claim", p).map(|_| ())
    }

    pub fn release(&mut self, room: &str, path: &str, note: Option<&str>) -> Result<()> {
        let mut p = json!({"room": room, "path": path});
        if let Some(n) = note {
            p["note"] = json!(n);
        }
        self.request("release", p).map(|_| ())
    }

    /// The owners of the live claims in a room, in path order.
    pub fn claim_owners(&mut self, room: &str) -> Result<Vec<String>> {
        let r = self.request("claims", json!({"room": room}))?;
        Ok(r["claims"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|c| c["owner"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Start (or resume) a stream; returns the room's head.
    pub fn subscribe(&mut self, room: &str, from_seq: u64) -> Result<u64> {
        self.caught_up = None;
        let r = self.request("subscribe", json!({"room": room, "from_seq": from_seq}))?;
        Ok(r["head"].as_u64().unwrap_or(0))
    }

    pub fn ack(&mut self, room: &str, seq: u64) -> Result<()> {
        self.send(&ClientFrame::Ack {
            v: PROTOCOL_VERSION,
            room: room.into(),
            seq,
        })
    }

    /// The messages received so far; with `until_caught_up`, wait for the replay to finish first.
    pub fn collect(&mut self, until_caught_up: bool, timeout: Duration) -> Result<Collected> {
        let end = Instant::now() + timeout;
        while until_caught_up && self.caught_up.is_none() && Instant::now() < end {
            match self.frame(end.saturating_duration_since(Instant::now()))? {
                Some(f) => self.note(f),
                None => break,
            }
        }
        Ok(Collected {
            messages: std::mem::take(&mut self.pending),
            caught_up: self.caught_up,
        })
    }

    /// The next message of a subscription, waiting for it; `None` on timeout.
    pub fn next_message(&mut self, timeout: Duration) -> Result<Option<Message>> {
        let end = Instant::now() + timeout;
        loop {
            if !self.pending.is_empty() {
                return Ok(Some(self.pending.remove(0)));
            }
            match self.frame(end.saturating_duration_since(Instant::now()))? {
                Some(f) => self.note(f),
                None => return Ok(None),
            }
        }
    }

    /// End the session by closing the input; returns the gateway's exit code.
    pub fn close(mut self) -> Result<i32> {
        drop(self.stdin.take());
        self.child
            .wait()
            .map(|s| s.code().unwrap_or(-1))
            .map_err(|e| Error::Session(e.to_string()))
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        drop(self.stdin.take());
        let _ = self.child.wait();
    }
}

fn message_of(m: &Value) -> Message {
    let text = |k: &str| m[k].as_str().unwrap_or("").to_string();
    Message {
        seq: m["seq"].as_u64().unwrap_or(0),
        id: text("id"),
        sender: text("sender"),
        attribution: text("attribution"),
        label: text("label"),
        body: text("body"),
        ts: m["ts"].as_i64().unwrap_or(0),
        kind: m["kind"].as_str().map(String::from),
        payload: m["payload"].as_str().map(String::from),
    }
}
