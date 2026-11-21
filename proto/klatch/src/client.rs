//! A thin klatch client over the wire protocol (PROTOCOL.md, version 1): it speaks newline-delimited JSON to
//! `klatch serve --stdio` using the same frame types the gateway does. Over stdio, who you are is decided by the
//! environment the process is started with, never by a request parameter. Over the daemon's Unix socket (`klatch-server
//! daemon`, #102; feature `client-socket`, #1095) the kernel reports the peer and the first request, `identify`, says
//! more (an [`Identity`]). Over `klatch-server http` (feature `client-http`, #1122) a bearer credential and the member name
//! say who you are, and the stream is server-sent events. A frame type or field this version does not know is ignored.

use crate::protocol::{
    Attribution, ClientFrame, PROTOCOL_VERSION, ServerFrame, decode_server_line, encode_line,
};
use crate::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use crate::sync::thread;
use serde_json::{Map, Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(all(unix, feature = "client-socket"))]
mod socket;

#[cfg(feature = "client-http")]
mod http;
#[cfg(feature = "client-http")]
pub use http::{HttpFault, HttpOptions};

/// A boxed writer for the session's input, and the action that ends a transport that has no child to wait for.
type Writer = Box<dyn Write + Send>;
type Shutdown = Box<dyn FnOnce() + Send>;

/// Who a connection over a daemon's socket says it is: the `identify` first request of the daemon (#102). Build it
/// with [`Identity::name`], [`Identity::credential`] or [`Identity::none`]. Nothing here is ever printed: a refusal
/// names a code, never the name or the token.
#[derive(Clone, PartialEq, Eq)]
pub struct Identity(Option<Value>);

impl Identity {
    /// A typed name, validated by the daemon as `KLATCH_NAME` is. The member is **unattributed**, as the CLI's is.
    pub fn name(name: &str) -> Self {
        Self(Some(json!({"name": name})))
    }

    /// A member and its credential token (the token only, never the key bundle file). A live credential gives a
    /// **verified** member; a bad one ends the connection with an error, and nothing falls back to a weaker name.
    pub fn credential(member: &str, token: &str) -> Self {
        Self(Some(json!({"member": member, "token": token})))
    }

    /// No `identify`: the member is what the daemon gives a local process nobody registered, `unmapped-process`,
    /// unattributed.
    pub fn none() -> Self {
        Self(None)
    }
}

impl std::fmt::Debug for Identity {
    /// Never the name or the token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match &self.0 {
            None => "Identity(none)",
            Some(_) => "Identity(..)",
        })
    }
}

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
    /// The sender was admitted in flag mode of the room's requirement (#990); set by `read` only. Display only.
    pub advisory: bool,
}

/// What `collect` returns.
#[derive(Debug, Default)]
pub struct Collected {
    pub messages: Vec<Message>,
    pub caught_up: Option<u64>,
}

/// What `history` returns: the messages, oldest first, and how many follow the last one returned.
#[derive(Debug, Default)]
pub struct History {
    pub messages: Vec<Message>,
    pub more: u64,
}

/// A member as `who` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub last_seen: i64,
    pub status: Option<String>,
    /// When the status was set (seconds), if known.
    pub status_ts: Option<i64>,
    pub kind: Option<String>,
    /// The repository id klatch observed for the member, if any (display only).
    pub repository: Option<String>,
    /// The short name of the member's starting directory, never a path.
    pub directory: Option<String>,
    /// The permission mode the member last reported (#677), if any hook has.
    pub mode: Option<String>,
}

/// A live claim as `claims` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimInfo {
    pub path: String,
    pub owner: String,
    pub reason: String,
    pub expires: i64,
    pub scope: Option<String>,
}

/// An item in the reviewer's queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewItem {
    pub id: i64,
    pub room: String,
    pub seq: u64,
    pub to_label: String,
    pub reason: String,
    pub text: String,
}

/// The reviewer's decision on a held item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict<'a> {
    Allow,
    /// Release this text in place of the held one.
    Redact(&'a str),
    /// Refuse it, with an optional reason.
    Deny(Option<&'a str>),
}

fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

fn opt_str(v: &Value) -> Option<String> {
    v.as_str().map(String::from)
}

fn array_of<T>(v: &Value, one: impl Fn(&Value) -> T) -> Vec<T> {
    v.as_array()
        .map(|a| a.iter().map(one).collect())
        .unwrap_or_default()
}

/// A typed handoff carried by a post: its kind and an opaque payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handoff<'a> {
    pub kind: &'a str,
    pub payload: Option<&'a str>,
}

/// What `Client::post` sends: the room and the text, and optionally a label and a handoff. Build it with
/// `PostRequest::new(room, body)` and the setters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostRequest<'a> {
    pub room: &'a str,
    pub body: &'a str,
    pub label: Option<&'a str>,
    pub handoff: Option<Handoff<'a>>,
}

impl<'a> PostRequest<'a> {
    pub fn new(room: &'a str, body: &'a str) -> Self {
        Self {
            room,
            body,
            label: None,
            handoff: None,
        }
    }

    /// Post under this label instead of the room's default.
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Make the post a typed handoff of this kind, with an optional payload.
    pub fn handoff(mut self, kind: &'a str, payload: Option<&'a str>) -> Self {
        self.handoff = Some(Handoff { kind, payload });
        self
    }
}

/// What `Client::claim` sends: the room and the path, and optionally a time to live and a reason. Build it with
/// `ClaimRequest::new(room, path)` and the setters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimRequest<'a> {
    pub room: &'a str,
    pub path: &'a str,
    pub ttl: Option<i64>,
    pub reason: Option<&'a str>,
}

impl<'a> ClaimRequest<'a> {
    pub fn new(room: &'a str, path: &'a str) -> Self {
        Self {
            room,
            path,
            ttl: None,
            reason: None,
        }
    }

    /// Hold the claim for this many seconds.
    pub fn ttl(mut self, secs: i64) -> Self {
        self.ttl = Some(secs);
        self
    }

    /// Say why the path is claimed.
    pub fn reason(mut self, reason: &'a str) -> Self {
        self.reason = Some(reason);
        self
    }
}

/// One session with the gateway. Not shared between threads: use one client per thread.
pub struct Client {
    pub member: String,
    pub attribution: String,
    pub clearance: String,
    /// The `klatch serve --stdio` child, when the session is one.
    child: Option<Child>,
    writer: Option<Writer>,
    /// Ends a transport with no child (a socket), once.
    shutdown: Option<Shutdown>,
    frames: Receiver<Option<std::result::Result<ServerFrame, String>>>,
    next_id: u64,
    refusals: Vec<(String, String)>,
    pending: Vec<Message>,
    caught_up: Option<u64>,
}

/// The reader thread's body: decode each line of the session's output and send it, then send `None` once at the end.
/// Split out so a test can run it under Shuttle (#406).
pub(crate) fn pump_frames<R: Read>(
    output: R,
    tx: Sender<Option<std::result::Result<ServerFrame, String>>>,
) {
    for line in BufReader::new(output).lines() {
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
        let stdin = child.stdin.take().map(|s| Box::new(s) as Writer);
        Self::start(stdout, stdin, Some(child), None, &Identity::none())
    }

    /// A session over a transport the caller opened: `reader` carries the gateway's frames, `writer` the requests,
    /// and `shutdown` ends the transport when the session is closed or dropped (a socket has no child to wait
    /// for). The hello is read, then `identity` is sent as the `identify` first request unless it is
    /// [`Identity::none`]. The socket connect of feature `client-socket` is this with a Unix socket.
    pub fn over<R, W>(reader: R, writer: W, shutdown: Shutdown, identity: &Identity) -> Result<Self>
    where
        R: Read + Send + 'static,
        W: Write + Send + 'static,
    {
        Self::start(
            reader,
            Some(Box::new(writer)),
            None,
            Some(shutdown),
            identity,
        )
    }

    fn start<R: Read + Send + 'static>(
        reader: R,
        writer: Option<Writer>,
        child: Option<Child>,
        shutdown: Option<Shutdown>,
        identity: &Identity,
    ) -> Result<Self> {
        let (tx, frames) = channel();
        thread::spawn(move || pump_frames(reader, tx));
        let mut c = Self {
            member: String::new(),
            attribution: String::new(),
            clearance: String::new(),
            child,
            writer,
            shutdown,
            frames,
            next_id: 0,
            refusals: Vec::new(),
            pending: Vec::new(),
            caught_up: None,
        };
        c.hello()?;
        if let Some(params) = &identity.0 {
            let r = c.request("identify", params.clone())?;
            if let Some(m) = r["member"].as_str() {
                c.member = m.to_string();
            }
            if let Some(a) = r["attribution"].as_str() {
                c.attribution = a.to_string();
            }
        }
        Ok(c)
    }

    fn hello(&mut self) -> Result<()> {
        let c = self;
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
                Ok(())
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
                advisory,
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
                advisory,
            }),
            ServerFrame::CaughtUp { seq, .. } => self.caught_up = Some(seq),
            ServerFrame::Refusal { rule, reason, .. } => self.refusals.push((rule, reason)),
            _ => {}
        }
    }

    fn send(&mut self, frame: &ClientFrame) -> Result<()> {
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| Error::Session("the session is closed".into()))?;
        writer
            .write_all(encode_line(frame).as_bytes())
            .and_then(|()| writer.flush())
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
                }) if rid == id => return self.outcome(ok, result, error),
                Some(other) => self.note(other),
            }
        }
    }

    /// The answer to a request: its result, or an `Error::Api` carrying the refusals seen meanwhile.
    fn outcome(
        &mut self,
        ok: bool,
        result: Option<Value>,
        error: Option<crate::protocol::ErrorBody>,
    ) -> Result<Value> {
        if ok {
            return Ok(result.unwrap_or(Value::Null));
        }
        let e = error.unwrap_or(crate::protocol::ErrorBody {
            code: "backend".into(),
            message: String::new(),
        });
        Err(Error::Api(ApiError {
            code: e.code,
            message: e.message,
            refusals: std::mem::take(&mut self.refusals),
        }))
    }

    /// Post a message; returns its `seq` and id.
    pub fn post(&mut self, request: PostRequest) -> Result<(u64, String)> {
        let mut p = Map::new();
        p.insert("room".into(), json!(request.room));
        p.insert("body".into(), json!(request.body));
        if let Some(l) = request.label {
            p.insert("label".into(), json!(l));
        }
        if let Some(Handoff { kind, payload }) = request.handoff {
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

    /// Send a direct message to one other member. Returns the two-member room made on first use (which both
    /// members can read, `history` and `subscribe` to), the `seq` and the `id`. The label may go below the
    /// room's ceiling (the lower of the two clearances), never above.
    pub fn dm(
        &mut self,
        to: &str,
        body: &str,
        label: Option<&str>,
    ) -> Result<(String, u64, String)> {
        let mut p = json!({"to": to, "body": body});
        if let Some(l) = label {
            p["label"] = json!(l);
        }
        let r = self.request("dm", p)?;
        Ok((
            r["room"].as_str().unwrap_or("").to_string(),
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

    /// Look back in a room without moving any cursor. No `last` and no `since`: the newest 20. `last`
    /// alone: the newest `last` (1 to 200). `since` alone: the next 20 after that `seq`. Both: the newest
    /// `last` after `since`. Page a long room by passing the last `seq` returned as the next `since`.
    pub fn history(
        &mut self,
        room: &str,
        last: Option<u32>,
        since: Option<u64>,
    ) -> Result<History> {
        let mut p = json!({"room": room});
        if let Some(n) = last {
            p["last"] = json!(n);
        }
        if let Some(s) = since {
            p["since"] = json!(s);
        }
        let r = self.request("history", p)?;
        Ok(History {
            messages: r["messages"]
                .as_array()
                .map(|a| a.iter().map(message_of).collect())
                .unwrap_or_default(),
            more: r["more"].as_u64().unwrap_or(0),
        })
    }

    /// Claim a path in a room.
    pub fn claim(&mut self, request: ClaimRequest) -> Result<()> {
        let mut p = json!({"room": request.room, "path": request.path});
        if let Some(t) = request.ttl {
            p["ttl"] = json!(t);
        }
        if let Some(r) = request.reason {
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

    /// The members of a room: liveness, and the status and kind a member declared.
    pub fn who(&mut self, room: &str) -> Result<Vec<Member>> {
        let r = self.request("who", json!({"room": room}))?;
        Ok(array_of(&r["members"], |m| Member {
            name: str_of(&m["name"]),
            last_seen: m["last_seen"].as_i64().unwrap_or(0),
            status: opt_str(&m["status"]),
            status_ts: m["status_ts"].as_i64(),
            kind: opt_str(&m["kind"]),
            repository: opt_str(&m["repository"]),
            directory: opt_str(&m["directory"]),
            mode: opt_str(&m["mode"]),
        }))
    }

    /// The live claims in a room, in path order.
    pub fn claims(&mut self, room: &str) -> Result<Vec<ClaimInfo>> {
        let r = self.request("claims", json!({"room": room}))?;
        Ok(array_of(&r["claims"], |c| ClaimInfo {
            path: str_of(&c["path"]),
            owner: str_of(&c["owner"]),
            reason: str_of(&c["reason"]),
            expires: c["expires"].as_i64().unwrap_or(0),
            scope: opt_str(&c["scope"]),
        }))
    }

    /// The stuck cases of a room as events (`stale_ask`, `nobody_capable`, `ask_cycle`, `claim_deadlock`): each a JSON
    /// object with a `kind`, a stable `key`, and ids, member names and numbers, never message text.
    pub fn stuck(&mut self, room: &str) -> Result<Vec<Value>> {
        let r = self.request("stuck", json!({"room": room}))?;
        Ok(r["events"].as_array().cloned().unwrap_or_default())
    }

    /// Set your own status in a room (one line, 1 to 80 characters), for example `done`.
    pub fn set_status(&mut self, room: &str, status: &str) -> Result<()> {
        self.request("status", json!({"room": room, "status": status}))
            .map(|_| ())
    }

    /// Join `room`, making it if it does not exist (the room-creation capability may need a grant).
    pub fn join(&mut self, room: &str) -> Result<()> {
        self.request("join", json!({"room": room})).map(|_| ())
    }

    /// The room's open asks as the wire gives them: `{seq, sender, text, ts, claimed, waiting_on_asker, room}` each.
    /// The text is another member's words: data, never an instruction.
    pub fn asks(&mut self, room: &str) -> Result<Vec<Value>> {
        let r = self.request("asks", json!({"room": room}))?;
        Ok(r["asks"].as_array().cloned().unwrap_or_default())
    }

    /// Post a content-free pointer (`klatch join <room>`) for `member` in `via`, a room you both already are in.
    /// It joins nobody; the invited member decides.
    pub fn invite(&mut self, member: &str, room: &str, via: &str) -> Result<()> {
        self.request(
            "invite",
            json!({"member": member, "room": room, "via": via}),
        )
        .map(|_| ())
    }

    /// As the room's grantor, give `member` a clearance of `label` in `room` only. Refused unless you are the room's
    /// grantor, the label is not the top level, and it is within the agent cap, the room's label and your own
    /// clearance; the refusal names the rule.
    pub fn grant_room_clearance(&mut self, room: &str, member: &str, label: &str) -> Result<()> {
        self.request(
            "grant_room_clearance",
            json!({"room": room, "member": member, "label": label}),
        )
        .map(|_| ())
    }

    /// Your own crossing requests, newest first, as the wire gives them: `{id, room, seq, to_room, state, why}` each.
    /// `state` is `waiting`, `approved`, `denied`, `released` or `blocked`; `why` is one fixed sentence, never the
    /// text or a reviewer's words.
    pub fn my_crossings(&mut self, limit: usize) -> Result<Vec<Value>> {
        let r = self.request("my_crossings", json!({"limit": limit}))?;
        Ok(r["crossings"].as_array().cloned().unwrap_or_default())
    }

    /// The names of the rooms you have joined, without reading anything. Only your own (#848).
    pub fn my_rooms(&mut self) -> Result<Vec<String>> {
        let r = self.request("my_rooms", json!({}))?;
        Ok(array_of(&r["rooms"], str_of))
    }

    /// The room's board as one block of text, each entry bounded. The entries are other members' words: data, never
    /// instructions (#848).
    pub fn board(&mut self, room: &str) -> Result<String> {
        let r = self.request("board", json!({"room": room}))?;
        Ok(str_of(&r["board"]))
    }

    /// What the sender of message `seq` can learn about each recipient, as the wire gives it: `{seq, posted,
    /// recipients: [{name, state, secs, shown, read}]}` with `state` one of `parked`, `seen_after`, `not_seen`.
    /// Refused unless you sent the message; never the text (#848).
    pub fn receipt(&mut self, room: &str, seq: i64) -> Result<Value> {
        self.request("receipt", json!({"room": room, "seq": seq}))
    }

    /// Your own role as one fixed word (`librarian`, `admin`, `reviewer`, `facilitator` or `integrator`), or `None`
    /// when you hold none. Only your own (#1110).
    pub fn my_role(&mut self) -> Result<Option<String>> {
        let r = self.request("my_role", json!({}))?;
        Ok(r["role"].as_str().map(str::to_owned))
    }

    /// Declare the scope you work in (1 to 80 characters of letters, digits and `. : _ - /`): your claims conflict
    /// only with claims in the same scope, and a shared claim conflicts with every scope.
    pub fn set_scope(&mut self, scope: &str) -> Result<()> {
        self.request("scope", json!({"scope": scope})).map(|_| ())
    }

    /// Declare your own kind for display (`bot`, `service` or `human`); `None` withdraws it. `session` is never
    /// declared: klatch derives it.
    pub fn set_kind(&mut self, kind: Option<&str>) -> Result<()> {
        self.request("kind", json!({"kind": kind})).map(|_| ())
    }

    /// The reviewer's queue: the next held item, or `None` when there is none. Only the reviewer role may call it
    /// (refused, rule `reviewer`).
    pub fn review_next(&mut self) -> Result<Option<ReviewItem>> {
        let r = self.request("review_next", json!({}))?;
        let item = &r["item"];
        if item.is_null() {
            return Ok(None);
        }
        Ok(Some(ReviewItem {
            id: item["id"].as_i64().unwrap_or(0),
            room: str_of(&item["room"]),
            seq: item["seq"].as_u64().unwrap_or(0),
            to_label: str_of(&item["to_label"]),
            reason: str_of(&item["reason"]),
            text: str_of(&item["text"]),
        }))
    }

    /// The reviewer's verdict on an item: `allow`, `redact` (with `text`) or `deny` (with an optional `reason`).
    /// Only the reviewer role may call it.
    pub fn review_verdict(&mut self, id: i64, verdict: Verdict) -> Result<()> {
        let mut p = json!({"id": id});
        match verdict {
            Verdict::Allow => p["verdict"] = json!("allow"),
            Verdict::Redact(text) => {
                p["verdict"] = json!("redact");
                p["text"] = json!(text);
            }
            Verdict::Deny(reason) => {
                p["verdict"] = json!("deny");
                if let Some(r) = reason {
                    p["reason"] = json!(r);
                }
            }
        }
        self.request("review_verdict", p).map(|_| ())
    }

    /// An act of the admin agent (`close`, `reopen`, `remove`, `allow`, `clear`, `label`, `audit`, `refusals`,
    /// `run-create`, `run-end`, `anchor-done`) with the parameters it needs (`room`, `member`, `label`, `limit`,
    /// `run`, `anchor`). A member without the role is refused. Returns the result object.
    pub fn role_act(
        &mut self,
        act: &str,
        mut params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        params["act"] = json!(act);
        self.request("role_act", params)
    }

    /// Housekeeping every member may run: remove the members not seen for the fixed retention. Returns how many.
    pub fn prune(&mut self) -> Result<u64> {
        let r = self.request("prune", json!({}))?;
        Ok(r["removed"].as_u64().unwrap_or(0))
    }

    /// Mute (or, with `muted` false, unmute) a room for yourself: your digest then shows only messages that
    /// mention you. `read` and the stream still carry everything.
    pub fn mute(&mut self, room: &str, muted: bool) -> Result<()> {
        self.request("mute", json!({"room": room, "muted": muted}))
            .map(|_| ())
    }

    /// Start (or resume) a stream; returns the room's head.
    pub fn subscribe(&mut self, room: &str, from_seq: u64) -> Result<u64> {
        self.caught_up = None;
        let r = self.request("subscribe", json!({"room": room, "from_seq": from_seq}))?;
        Ok(r["head"].as_u64().unwrap_or(0))
    }

    /// Stop the stream for a room; harmless if there is none.
    pub fn unsubscribe(&mut self, room: &str) -> Result<()> {
        self.request("unsubscribe", json!({"room": room}))
            .map(|_| ())
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
        while self.replaying(until_caught_up, end) {
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

    /// Whether `collect` should keep waiting for the replay to finish before `end`.
    fn replaying(&self, until_caught_up: bool, end: Instant) -> bool {
        until_caught_up && self.caught_up.is_none() && Instant::now() < end
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

    /// End the session by closing the input; returns the gateway's exit code. Over a socket there is no child:
    /// the socket is shut and the daemon's end of the session is awaited (up to 10 seconds), and the answer is 0.
    pub fn close(mut self) -> Result<i32> {
        self.end_input();
        if let Some(child) = self.child.as_mut() {
            return child
                .wait()
                .map(|s| s.code().unwrap_or(-1))
                .map_err(|e| Error::Session(e.to_string()));
        }
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            match self
                .frames
                .recv_timeout(end.saturating_duration_since(Instant::now()))
            {
                Ok(Some(_)) => {}
                Ok(None) | Err(RecvTimeoutError::Disconnected) => return Ok(0),
                Err(RecvTimeoutError::Timeout) => {
                    return Err(Error::Session("the daemon did not end the session".into()));
                }
            }
        }
    }

    /// Close the input: drop the writer, and end a transport that has no child.
    fn end_input(&mut self) {
        drop(self.writer.take());
        if let Some(shutdown) = self.shutdown.take() {
            shutdown();
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.end_input();
        if let Some(child) = self.child.as_mut() {
            let _ = child.wait();
        }
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
        advisory: m["advisory"].as_bool().unwrap_or(false),
    }
}
