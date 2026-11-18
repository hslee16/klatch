//! The session module (the `klatch-session` crate before the crates became modules): one gateway session over any duplex byte stream, speaking `PROTOCOL.md`.
//! Identity comes from the `Principal` the transport supplies, never from a request.

mod frames;
mod guarded;
mod methods;
pub mod policy;
mod role_methods;

use crate::domain::Principal;
use crate::gateway::{Connection, Gateway, Identity, Subscription};
use crate::protocol::{ClientFrame, DecodeError, FrameReader, PROTOCOL_VERSION, ServerFrame};
use crate::store::Store;
use crate::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError, channel};
use crate::sync::thread;
use frames::{Out, attribution, decode_refusal, failure, message_frame, ok, stream_refusal};
pub use guarded::Closer;
use methods::{Call, RoomParams, handle, params};
use serde::Deserialize;
use serde_json::json;
use std::io::{self, BufRead, Write};
use std::time::Duration;

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
pub struct Session<'a, S: Store> {
    gateway: &'a Gateway<S>,
    who: &'a Principal,
    config: &'a SessionConfig,
}

impl<'a, S: Store> Session<'a, S> {
    pub fn new(gateway: &'a Gateway<S>, who: &'a Principal, config: &'a SessionConfig) -> Self {
        Self {
            gateway,
            who,
            config,
        }
    }

    fn hello<W: Write>(&self, out: &mut Out<W>) -> io::Result<()> {
        out.send(&ServerFrame::Hello {
            v: PROTOCOL_VERSION,
            member: self.who.member.clone(),
            attribution: attribution(self.who.attribution),
            clearance: DEFAULT_CLEARANCE.into(),
        })
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
        self.hello(&mut out)?;
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
        self.hello(&mut out)?;
        let from = i64::try_from(request.from_seq).unwrap_or(i64::MAX);
        let mut subs = vec![Subscription::new(request.room, from, usize::MAX / 2)];
        loop {
            // taken before the poll, so a commit after it ends the pause below at once (#148)
            let seen = self.gateway.change_token();
            self.pump(&mut subs, &mut out)?;
            if subs.is_empty() {
                return Err(io::Error::other("the stream ended after a refusal"));
            }
            if !request.follow && subs[0].is_caught_up() {
                return Ok(());
            }
            if request.follow && out.last.elapsed() >= self.config.heartbeat {
                self.heartbeat(&mut out)?;
            }
            crate::gateway::pause_for_change(
                &|| self.gateway.change_token(),
                seen,
                self.config.poll,
                crate::gateway::WAKE_TICK.min(self.config.poll),
            );
        }
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
            let batch = match sub.poll(self.gateway, self.who, self.config.batch) {
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
            _ => handle(self.gateway, self.who, call),
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
    match rx.recv_timeout(wait) {
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
mod role_tests;
#[cfg(test)]
mod run_act_tests;
#[cfg(test)]
mod tests;
