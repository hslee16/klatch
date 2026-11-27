//! The session module (the `klatch-session` crate before the crates became modules): one gateway session over any duplex byte stream, speaking `PROTOCOL.md`.
//! Identity comes from the `Principal` the transport supplies, never from a request.

pub mod acp;
pub(crate) mod acp_reader;
mod backend;
pub(crate) mod cancellable;
mod fixed;
mod frames;
mod guarded;
pub mod mcp;
mod methods;
pub mod policy;
mod role_methods;

use crate::domain::Principal;
use crate::gateway::{Connection, Gateway, Identity, Subscription};
use crate::protocol::{ClientFrame, DecodeError, FrameReader, PROTOCOL_VERSION, ServerFrame};
use crate::store::Store;
use crate::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError, channel};
use crate::sync::thread;
pub use backend::{Backend, CancelHook, NoCancel, Outcome};
use frames::{Out, attribution, decode_refusal, failure, message_frame, ok, stream_refusal};
#[doc(hidden)]
pub use methods::Call;

/// The frames that answer a request that failed with `e` (a `response` with the error).
#[doc(hidden)]
pub fn failure_frames(id: String, e: &crate::domain::Error) -> Vec<ServerFrame> {
    failure(id, e)
}
pub use guarded::Closer;
use methods::{RoomParams, params};
use serde::Deserialize;
use serde_json::json;
use std::io::{self, BufRead, Write};
use std::time::Duration;

/// Until the label model exists every caller has the lowest clearance; this is the safe direction.
const DEFAULT_CLEARANCE: &str = "public";

/// The `hello` a session sends for a principal. A transport that settles who the peer is after the session has begun
/// (the daemon's `identify`, #102) sends it itself and starts the session `without_hello`.
#[doc(hidden)]
pub fn hello_frame(who: &Principal) -> ServerFrame {
    ServerFrame::Hello {
        v: PROTOCOL_VERSION,
        member: who.member.clone(),
        attribution: attribution(who.attribution),
        clearance: DEFAULT_CLEARANCE.into(),
    }
}

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
    /// Frames a guarded session may hold for a consumer that is slow to read (#147).
    pub outgoing: usize,
    /// How long a guarded session waits for a full outgoing queue to move before it closes the session with a
    /// `stream.slow_consumer` refusal (#147).
    pub stall: Duration,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            poll: Duration::from_millis(100),
            heartbeat: Duration::from_secs(15),
            window: crate::gateway::DEFAULT_WINDOW,
            batch: 64,
            outgoing: 1024,
            stall: Duration::from_secs(30),
        }
    }
}

/// Run a session with the default settings; see `Session::serve`.
pub fn serve<S: Store, R: BufRead + Send + 'static, W: Write>(
    gateway: &Gateway<S>,
    who: &Principal,
    input: R,
    output: W,
) -> io::Result<()> {
    Session::new(gateway, who, &SessionConfig::default()).serve(input, output)
}

/// Serve one transport connection: the identity maps the peer's evidence to a principal (an unmapped peer comes
/// out unattributed, R23) and the session loop runs over the connection's stream. The peer never names itself.
pub fn serve_connection<S: Store>(
    gateway: &Gateway<S>,
    identity: &dyn Identity,
    connection: Connection,
    config: &SessionConfig,
) -> io::Result<()> {
    let who = identity
        .resolve(&connection.evidence)
        .map_err(io::Error::other)?;
    Session::new(gateway, &who, config).serve_guarded(
        connection.input,
        connection.output,
        connection.closer,
    )
}

/// What `Session::stream` writes: the history of one room after `from_seq`, and with `follow` its live messages.
pub struct StreamRequest<'a> {
    pub room: &'a str,
    pub from_seq: u64,
    pub follow: bool,
}

/// A request or acknowledgement read from the input, or why it could not be read.
type Item = Result<ClientFrame, DecodeError>;

/// One session: a gateway, the principal it acts for and its settings.
pub struct Session<'a, B: Backend + ?Sized> {
    gateway: &'a B,
    who: &'a Principal,
    config: &'a SessionConfig,
    /// The transport fixed the member (an ssh forced command, #107): a request that claims an identity is refused.
    fixed_member: bool,
    /// A look at what the session is bound to (#1083, an ssh key mapping): taken before each batch of requests and on
    /// every tick; an error ends the session with that refusal.
    recheck: Option<&'a dyn Fn() -> crate::domain::Result<()>>,
    /// Whether the session sends `hello` itself (a transport that already told the peer who it is turns this off).
    greet: bool,
    /// The harness the transport edge verified for this process (#1115), which a `join` is judged by; `None` is the
    /// generic floor. Never read from a request.
    harness: Option<&'a dyn crate::harness::Harness>,
}

impl<'a, S: Store> Session<'a, Gateway<S>> {
    /// A session over a gateway in this thread.
    pub fn new(gateway: &'a Gateway<S>, who: &'a Principal, config: &'a SessionConfig) -> Self {
        Self::over(gateway, who, config)
    }
}

impl<'a, B: Backend + ?Sized> Session<'a, B> {
    /// A session over any backend: a gateway in this thread, or a handle to one that another thread owns (#102).
    pub fn over(gateway: &'a B, who: &'a Principal, config: &'a SessionConfig) -> Self {
        Self {
            gateway,
            who,
            config,
            fixed_member: false,
            recheck: None,
            greet: true,
            harness: None,
        }
    }

    /// Judge a `join` by `harness`, which the caller's edge verified from evidence of its own (the process tree, as
    /// for a CLI command, #877), never from anything a request says (#1115). Without it a `join` is the generic floor.
    #[must_use]
    pub fn with_harness(mut self, harness: &'a dyn crate::harness::Harness) -> Self {
        self.harness = Some(harness);
        self
    }

    /// Mark the member as fixed by the transport: a request with a parameter that claims who is calling (`sender`,
    /// `from`, `as`, `identity`, `attribution`, or a `member` where the method has no member target) is refused with the
    /// rule `member-fixed` and is not run. The member was never taken from a request; this makes an attempt visible.
    pub fn fixed_member(mut self) -> Self {
        self.fixed_member = true;
        self
    }

    /// Look at `check` before each batch of requests and on every tick of an idle session (#1083). When it returns an
    /// error the requests that arrived are answered with that error, a refusal naming no request is sent, and the
    /// session ends. An ssh session uses it so that unmapping its key ends it; `Busy` is a store that was momentarily
    /// locked and is looked at again on the next tick.
    #[must_use]
    pub fn rechecked(mut self, check: &'a dyn Fn() -> crate::domain::Result<()>) -> Self {
        self.recheck = Some(check);
        self
    }

    /// The reason the session's binding is gone, if it is.
    fn binding_lost(&self) -> Option<crate::domain::Error> {
        match self.recheck?() {
            Err(crate::domain::Error::Busy(_)) | Ok(()) => None,
            Err(e) => Some(e),
        }
    }

    /// Do not send `hello` when the session starts: the transport already sent one and settled the principal.
    #[must_use]
    pub fn without_hello(mut self) -> Self {
        self.greet = false;
        self
    }

    fn hello<W: Write>(&self, out: &mut Out<W>) -> io::Result<()> {
        out.send(&hello_frame(self.who))
    }

    fn heartbeat<W: Write>(&self, out: &mut Out<W>) -> io::Result<()> {
        out.send(&ServerFrame::Heartbeat {
            v: PROTOCOL_VERSION,
            ts: self.gateway.now(),
        })
    }

    /// Run a session: send `hello`, answer every request with exactly one `response`, and push stream frames
    /// for subscribed rooms, until the input ends.
    ///
    /// Input is read on a helper thread while this thread handles requests, polls the store for new messages,
    /// pushes them and sends heartbeats when idle. When the input ends the session delivers whatever replay is
    /// immediately available (within each subscription's window) and returns.
    pub fn serve<R: BufRead + Send + 'static, W: Write>(
        &self,
        input: R,
        output: W,
    ) -> io::Result<()> {
        let mut out = Out::new(output);
        if self.greet {
            self.hello(&mut out)?;
        }
        self.run(&read_in_background(input), &mut out)
    }

    /// The session loop over an input already being read and an output that has had its `hello`.
    fn run<W: Write>(&self, rx: &Receiver<Item>, out: &mut Out<W>) -> io::Result<()> {
        let mut subs: Vec<Subscription> = Vec::new();
        let mut open = true;
        // The store is polled when a request came in, the store's change token moved, or the poll period ran out:
        // an idle session looks at one cheap token per tick instead of querying every room (#148).
        let mut pacer = Pacer::new(self.gateway.change_token(), self.config.poll);
        while open {
            let items = collect(rx, pacer.tick, &mut open);
            let forced = !items.is_empty() || !open;
            // an output that has gone away ends the session even when it has nothing to send (#146)
            out.healthy()?;
            if let Some(lost) = self.binding_lost() {
                return refuse_lost(items, &lost, out);
            }
            for item in items {
                self.dispatch(item, &mut subs, out)?;
            }
            if pacer.due(self.gateway.change_token(), forced) {
                self.deliver(&mut subs, out, open)?;
            }
            if open && out.last.elapsed() >= self.config.heartbeat {
                self.heartbeat(out)?;
            }
        }
        Ok(())
    }

    /// Poll the subscriptions once; with the input closed, keep going while replay is immediately available.
    fn deliver<W: Write>(
        &self,
        subs: &mut Vec<Subscription>,
        out: &mut Out<W>,
        open: bool,
    ) -> io::Result<()> {
        self.pump(subs, out)?;
        while !open && self.pump(subs, out)? > 0 {}
        Ok(())
    }

    /// Answer one item read from the input.
    fn dispatch<W: Write>(
        &self,
        item: Item,
        subs: &mut Vec<Subscription>,
        out: &mut Out<W>,
    ) -> io::Result<()> {
        match item {
            Ok(ClientFrame::Request {
                id, method, params, ..
            }) => {
                if self.fixed_member
                    && let Some(field) = fixed::identity_claim(&method, &params)
                {
                    self.gateway.note_member_fixed(self.who);
                    return out.send_all(&frames::failure(id, &member_fixed_refusal(&field)));
                }
                let call = Call {
                    id,
                    method,
                    value: params,
                };
                out.send_all(&self.request(subs, call))
            }
            Ok(ClientFrame::Ack { room, seq, .. }) => {
                out.send_all(&self.acknowledge(subs, &room, seq))
            }
            Ok(ClientFrame::Unknown) => Ok(()),
            Err(DecodeError::Io(e)) => Err(io::Error::other(e)),
            Err(e) => out.send(&decode_refusal(&e)),
        }
    }

    /// Write the event stream of one room to `output` with no request channel: `hello`, the history after
    /// `from_seq`, `caught_up`, and, with `follow`, live messages and heartbeats until the output closes.
    ///
    /// This is the daemon-free producer behind `klatch stream`. A pipe reader cannot acknowledge, so there is
    /// no flow-control window. The room is checked before anything is written, so a refused or invalid room
    /// leaves the output empty and returns an error.
    pub fn stream<W: Write>(&self, request: &StreamRequest, output: W) -> io::Result<()> {
        self.gateway
            .head(self.who, request.room)
            .map_err(io::Error::other)?;
        let mut out = Out::new(output);
        if self.greet {
            self.hello(&mut out)?;
        }
        let from = i64::try_from(request.from_seq).unwrap_or(i64::MAX);
        let mut subs = vec![Subscription::new(request.room, from, usize::MAX / 2)];
        loop {
            // taken before the poll, so a commit after it ends the pause below at once (#148)
            let seen = self.gateway.change_token();
            if self.stream_step(request, &mut subs, &mut out)? {
                return Ok(());
            }
            crate::gateway::pause_for_change(
                &|| self.gateway.change_token(),
                seen,
                self.config.poll,
                crate::gateway::WAKE_TICK.min(self.config.poll),
            );
        }
    }

    /// One poll of a stream; true when the stream is finished (caught up without `follow`).
    fn stream_step<W: Write>(
        &self,
        request: &StreamRequest,
        subs: &mut Vec<Subscription>,
        out: &mut Out<W>,
    ) -> io::Result<bool> {
        self.pump(subs, out)?;
        if subs.is_empty() {
            return Err(io::Error::other("the stream ended after a refusal"));
        }
        if !request.follow && subs[0].is_caught_up() {
            return Ok(true);
        }
        if request.follow && out.last.elapsed() >= self.config.heartbeat {
            self.heartbeat(out)?;
        }
        Ok(false)
    }

    /// Handle an acknowledgement: move the subscription's window and the member's read cursor.
    fn acknowledge(&self, subs: &mut [Subscription], room: &str, seq: u64) -> Vec<ServerFrame> {
        let seq = i64::try_from(seq).unwrap_or(i64::MAX);
        if let Some(sub) = subs.iter_mut().find(|s| s.room() == room) {
            sub.ack(seq);
        }
        match self.gateway.ack(self.who, room, seq) {
            Ok(()) => Vec::new(),
            Err(e) => stream_refusal(&e).into_iter().collect(),
        }
    }

    /// Poll every subscription once (draining a room while batches come back full) and send what they yield.
    /// Returns how many frames were sent. A subscription that fails is dropped after a refusal.
    fn pump<W: Write>(&self, subs: &mut Vec<Subscription>, out: &mut Out<W>) -> io::Result<usize> {
        let mut sent = 0;
        let mut dropped: Vec<String> = Vec::new();
        for sub in subs.iter_mut() {
            let (n, failed) = self.drain(sub, out)?;
            sent += n;
            if failed {
                dropped.push(sub.room().to_string());
            }
        }
        subs.retain(|s| !dropped.iter().any(|r| r == s.room()));
        Ok(sent)
    }

    /// Poll one subscription until a batch comes back short; the frames sent, and whether it failed with a refusal.
    fn drain<W: Write>(
        &self,
        sub: &mut Subscription,
        out: &mut Out<W>,
    ) -> io::Result<(usize, bool)> {
        let mut sent = 0;
        loop {
            let batch = match self.gateway.poll(sub, self.who, self.config.batch) {
                Ok(batch) => batch,
                Err(e) => {
                    let Some(frame) = stream_refusal(&e) else {
                        return Ok((sent, false));
                    };
                    out.send(&frame)?;
                    return Ok((sent + 1, true));
                }
            };
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
            if batch.messages.len() < self.config.batch {
                return Ok((sent, false));
            }
        }
    }

    /// One plain request outside a stream, as the session's principal: the frames that answer it (a `response`, and a
    /// `refusal` before it when a policy refused). Subscriptions are not available here. The MCP and ACP front ends call
    /// through this, so it meets exactly the checks a wire request meets (#299).
    pub fn call(&self, method: &str, params: serde_json::Value) -> Vec<ServerFrame> {
        let call = Call {
            id: "call".into(),
            method: method.into(),
            value: params,
        };
        self.request(&mut Vec::new(), call)
    }

    /// The count behind the one-time never-read notice (#1415), for the session's principal; zero when it does not apply
    /// or cannot be counted. Moves no cursor.
    pub fn first_post_unread(&self, room: &str) -> usize {
        self.gateway.first_post_unread(self.who, room).unwrap_or(0)
    }

    /// Dispatch one request. `subscribe` and `unsubscribe` change the session's subscriptions; every other
    /// method is a plain call through the gateway.
    fn request(&self, subs: &mut Vec<Subscription>, call: Call) -> Vec<ServerFrame> {
        match call.method.as_str() {
            "subscribe" => self.subscribe(subs, call),
            "unsubscribe" => match params::<RoomParams>(&call.id, call.value) {
                Ok(p) => {
                    subs.retain(|s| s.room() != p.room);
                    vec![ok(call.id, json!({}))]
                }
                Err(frames) => frames,
            },
            _ => self.gateway.handle_as(self.who, call, self.harness),
        }
    }

    fn subscribe(&self, subs: &mut Vec<Subscription>, call: Call) -> Vec<ServerFrame> {
        #[derive(Deserialize)]
        struct SubscribeParams {
            room: String,
            from_seq: u64,
        }
        let id = call.id;
        let p = match params::<SubscribeParams>(&id, call.value) {
            Ok(p) => p,
            Err(frames) => return frames,
        };
        match self.gateway.head(self.who, &p.room) {
            Ok(head) => {
                subs.retain(|s| s.room() != p.room);
                let from = i64::try_from(p.from_seq).unwrap_or(i64::MAX);
                subs.push(Subscription::new(&p.room, from, self.config.window));
                vec![ok(id, json!({ "head": head }))]
            }
            Err(e) => failure(id, &e),
        }
    }
}

/// The refusal of a request that claims an identity on a session whose member is fixed. It names the field and never
/// repeats the name that was claimed.
fn member_fixed_refusal(field: &str) -> crate::domain::Error {
    crate::domain::Error::Refused {
        rule: "member-fixed".into(),
        reason: format!(
            "this session's member is fixed by its ssh key; the request field `{field}` claims an identity and is refused; \
             drop it, the member is whoever the key is mapped to"
        ),
    }
}

/// When a session looks at the store: on a request, when the store's change token moved, or when the poll period ran
/// out. A store with no token is looked at on the period alone, as before (#148).
struct Pacer {
    /// How long the session waits for input between looks at the token.
    tick: Duration,
    seen: Option<u64>,
    last_poll: std::time::Instant,
    period: Duration,
}

impl Pacer {
    fn new(token: Option<u64>, period: Duration) -> Self {
        let tick = match token {
            Some(_) => period.min(crate::gateway::WAKE_TICK),
            None => period,
        };
        Self {
            tick,
            seen: token,
            last_poll: std::time::Instant::now(),
            period,
        }
    }

    /// Whether to poll now. When it says yes the token is recorded here, before the poll, so a commit after it shows
    /// as a change on the next tick.
    fn due(&mut self, token: Option<u64>, forced: bool) -> bool {
        let due = forced || token != self.seen || self.last_poll.elapsed() >= self.period;
        if due {
            self.seen = token;
            self.last_poll = std::time::Instant::now();
        }
        due
    }
}

/// Read the input on a helper thread, so the session can handle requests and poll the store meanwhile.
pub(crate) fn read_in_background<R: BufRead + Send + 'static>(input: R) -> Receiver<Item> {
    spawn_reader(input).0
}

/// The same, with the handle of the reader thread, for a session that can stop the input and join the thread (#146).
pub(crate) fn spawn_reader<R: BufRead + Send + 'static>(
    input: R,
) -> (Receiver<Item>, thread::JoinHandle<()>) {
    let (tx, rx) = channel();
    let handle = thread::spawn(move || {
        let mut reader = FrameReader::new(input);
        while let Some(item) = reader.next_client() {
            if tx.send(item).is_err() {
                break;
            }
        }
    });
    (rx, handle)
}

/// What arrived within `wait`, and (in `open`) whether the input is still open.
pub(crate) fn collect(rx: &Receiver<Item>, wait: Duration, open: &mut bool) -> Vec<Item> {
    let mut items = Vec::new();
    match crate::sync::recv_within(rx, wait) {
        Ok(item) => items.push(item),
        Err(RecvTimeoutError::Timeout) => {}
        Err(RecvTimeoutError::Disconnected) => *open = false,
    }
    while *open {
        match rx.try_recv() {
            Ok(item) => items.push(item),
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => *open = false,
        }
    }
    items
}

#[cfg(test)]
mod acp_cancel_tests;
#[cfg(test)]
mod acp_tests;
#[cfg(test)]
mod cancel_effect_tests;
#[cfg(test)]
mod cancel_hook_tests;
#[cfg(test)]
mod facilitator_tests;
#[cfg(test)]
mod never_read_tests;
#[cfg(test)]
mod role_tests;
#[cfg(test)]
mod run_act_tests;
#[cfg(test)]
mod tests;

/// A session whose binding is lost answers every request still waiting with the refusal, then ends with the stream's own.
fn refuse_lost<W: Write>(
    items: Vec<Item>,
    lost: &crate::domain::Error,
    out: &mut Out<W>,
) -> io::Result<()> {
    for item in items {
        if let Ok(ClientFrame::Request { id, .. }) = item {
            out.send_all(&failure(id, lost))?;
        }
    }
    if let Some(frame) = stream_refusal(lost) {
        out.send(&frame)?;
    }
    Ok(())
}
