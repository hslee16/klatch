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

mod methods;
mod records;
mod requests;
#[cfg(all(unix, feature = "client-socket"))]
mod socket;

#[cfg(feature = "client-http")]
mod http;
#[cfg(feature = "client-http")]
pub use http::{HttpFault, HttpOptions};

pub use records::{ClaimInfo, Collected, History, Member, Message, ReviewItem, Verdict};
use records::{array_of, message_of, opt_str, str_of};
pub use requests::{ClaimRequest, Handoff, PostRequest, RefInput};

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

/// The ways a session's input ends: its writer, the child to wait for, and the action that ends a transport with no child.
struct Link {
    writer: Option<Writer>,
    child: Option<Child>,
    shutdown: Option<Shutdown>,
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
        let link = Link {
            writer: stdin,
            child: Some(child),
            shutdown: None,
        };
        Self::start(stdout, link, &Identity::none())
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
        let link = Link {
            writer: Some(Box::new(writer)),
            child: None,
            shutdown: Some(shutdown),
        };
        Self::start(reader, link, identity)
    }

    fn start<R: Read + Send + 'static>(reader: R, link: Link, identity: &Identity) -> Result<Self> {
        let Link {
            writer,
            child,
            shutdown,
        } = link;
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
                scope,
                refs,
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
                scope,
                refs,
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
