//! A gateway shared by several threads (#140): one owner thread holds the `Gateway`, connection threads send it
//! requests over a channel.
//!
//! The gateway is single-threaded on purpose: it holds an `Rc`, the SQLite connection (one writer, not `Sync`), the
//! label keys in a `RefCell`, and boxed clock, guard, notifier and lease source with no `Send` bound. Rather than make
//! all of that `Send + Sync` and put a lock around it, the `Gateway` is built on the owner thread and never leaves it:
//! the type needs no change, the store stays one connection and one writer, and a request is one whole call, so the
//! guard, the clock and the store see exactly the order they see today. A handle (`SharedTransport`) is `Clone`,
//! `Send` and `Sync` and implements `Transport`, so a consumer works unchanged against it.
//!
//! The owner serves one request at a time and never waits for anything: `Transport` has no blocking call. A `wait` is a
//! loop in the caller of short `wait_poll` requests and sleeps on the caller's thread, so a wait holds nothing. The
//! change token the loop sleeps on is the store's own plus a count of posts made through the handles (the store's
//! counter does not move for the owner's own commits), so an in-process wait wakes on a post at once.
//! The `klatch` binary keeps building its own `Direct` over its own gateway and does not use this module.

use crate::domain::{Error, Principal, Result};
use crate::gateway::{Batch, Direct, Gateway, HistoryRange, Subscription, Transport};
use crate::protocol::ServerFrame;
use crate::session::{Backend, Call};
use crate::store::Store;
use crate::sync::{mpsc, thread};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Makes the harness a connection's `join` is judged by. Called on the owner thread, so the harness need not be `Send`.
type HarnessMaker = Arc<dyn Fn() -> Box<dyn crate::harness::Harness> + Send + Sync>;

/// What the owner runs for one request: given the transport bound to the request's principal, do the call and send
/// the answer back.
type Run = Box<dyn FnOnce(&dyn Transport) + Send>;

/// Loads the keys of the sealed labels named (the ones one member's clearance reaches), called on the owner thread for
/// each request. A label it has no key for stays sealed. Never given a label the member's clearance does not reach.
type KeyLoader = Arc<dyn Fn(&[String]) -> Result<crate::keys::LabelKeys> + Send + Sync>;

/// What the owner runs for one session-backend request: the gateway itself, as the session loop sees it, and the
/// harness of the connection when its edge verified one.
type BackendRun = Box<dyn FnOnce(&dyn Backend, Option<&dyn crate::harness::Harness>) + Send>;

enum Work {
    /// A `Transport` call as one principal.
    Transport(Run),
    /// A call of the session loop (`session::Backend`).
    Backend(BackendRun),
}

struct Job {
    principal: Principal,
    /// Whether the label keys of `principal`'s clearance are loaded for this request (the key loader, when there is one).
    keyed: bool,
    harness: Option<HarnessMaker>,
    run: Work,
}

/// The keys `member`'s clearance reaches, from `loader`; none when it reaches no sealed label or the loader fails
/// (fails closed: a sealed label stays sealed).
fn ring_for<S: Store>(
    gateway: &Gateway<S>,
    loader: &KeyLoader,
    member: &str,
) -> crate::keys::LabelKeys {
    let labels = gateway.sealed_labels_within(member).unwrap_or_default();
    if labels.is_empty() {
        return crate::keys::LabelKeys::new();
    }
    loader(&labels).unwrap_or_default()
}

/// The owner thread's join handle; joined when the last handle goes away.
struct Owner {
    thread: Option<thread::JoinHandle<()>>,
}

impl Drop for Owner {
    fn drop(&mut self) {
        // Every sender is gone by now (the senders are fields declared before this one), so the owner's loop has
        // ended or is about to. Joining makes the store connection close before the last handle returns.
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A handle to the thread that owns the gateway. `Clone`, `Send` and `Sync`: give one clone to each connection.
#[derive(Clone)]
pub struct SharedGateway {
    tx: mpsc::Sender<Job>,
    pub(in crate::gateway) posts: Arc<AtomicU64>,
    _owner: Arc<Owner>,
}

impl SharedGateway {
    /// Start the owner thread and build the gateway on it with `build`, so nothing in the gateway has to be `Send`
    /// (open the store inside `build`). Returns what `build` failed with, or `Backend` if it panicked.
    pub fn spawn<S: Store + 'static>(
        build: impl FnOnce() -> Result<Gateway<S>> + Send + 'static,
    ) -> Result<Self> {
        Self::start(build, None)
    }

    /// `spawn`, with a key source (#1096): for each request the owner asks `keys` for the keys of the sealed labels the
    /// request's principal is cleared to (never another label), puts them on the store for that one request, and
    /// takes them off after it, so a connection holds no key of another member and none is kept between requests.
    /// `keys` reads the keystore afresh each time, so a rotated key is seen at once.
    pub fn spawn_keyed<S: Store + 'static>(
        build: impl FnOnce() -> Result<Gateway<S>> + Send + 'static,
        keys: impl Fn(&[String]) -> Result<crate::keys::LabelKeys> + Send + Sync + 'static,
    ) -> Result<Self> {
        Self::start(build, Some(Arc::new(keys)))
    }

    fn start<S: Store + 'static>(
        build: impl FnOnce() -> Result<Gateway<S>> + Send + 'static,
        loader: Option<KeyLoader>,
    ) -> Result<Self> {
        let (tx, rx) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        let thread = thread::spawn(move || {
            let gateway = match catch_unwind(AssertUnwindSafe(build)) {
                Ok(Ok(g)) => Rc::new(g),
                Ok(Err(e)) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
                Err(_) => {
                    let _ =
                        ready_tx.send(Err(Error::Backend("building the gateway panicked".into())));
                    return;
                }
            };
            let _ = ready_tx.send(Ok(()));
            while let Ok(job) = rx.recv() {
                // The keys of this request's principal, and no one else's, for this request only (#1096).
                let keyed = job.keyed.then_some(()).and(loader.as_ref());
                if let Some(loader) = keyed {
                    gateway.use_label_keys(ring_for(&gateway, loader, &job.principal.member));
                }
                // A panic in one request fails that request (its reply channel closes) and nothing else: SQLite
                // rolls back what was open, and the next request starts clean.
                match job.run {
                    Work::Transport(run) => {
                        let mut direct = Direct::new(gateway.clone(), job.principal);
                        if let Some(make) = &job.harness {
                            direct = direct.with_harness(make());
                        }
                        let _ = catch_unwind(AssertUnwindSafe(|| run(&direct)));
                    }
                    Work::Backend(run) => {
                        let harness = job.harness.as_ref().map(|make| make());
                        let _ =
                            catch_unwind(AssertUnwindSafe(|| run(&*gateway, harness.as_deref())));
                    }
                }
                if keyed.is_some() {
                    gateway.use_label_keys(crate::keys::LabelKeys::new());
                }
            }
        });
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                tx,
                posts: Arc::new(AtomicU64::new(0)),
                _owner: Arc::new(Owner {
                    thread: Some(thread),
                }),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err(Error::Backend(
                "the gateway thread ended before it was ready".into(),
            )),
        }
    }

    /// A transport bound to `principal`, the way `Direct` is: callers never name who they are per call.
    pub fn transport(&self, principal: Principal) -> SharedTransport {
        SharedTransport {
            gateway: self.clone(),
            principal,
            harness: None,
        }
    }

    /// What the session loop calls (`session::Backend`), served by the owner thread: with it `session::Session::over`
    /// runs the protocol of `klatch serve --stdio` for any number of connections over this one gateway (#102).
    pub fn session_backend(&self) -> SharedBackend {
        SharedBackend {
            gateway: self.clone(),
            harness: None,
        }
    }

    /// Run `f` on the owner thread against the gateway itself.
    fn call_backend<R: Send + 'static>(
        &self,
        f: impl FnOnce(&dyn Backend) -> R + Send + 'static,
    ) -> Result<R> {
        self.call_backend_as((None, &None), move |b, _| f(b))
    }

    /// `call_backend` for a request made as `who` on a connection with `harness` (its keys and its harness, #1096).
    fn call_backend_as<R: Send + 'static>(
        &self,
        (who, harness): (Option<&Principal>, &Option<HarnessMaker>),
        f: impl FnOnce(&dyn Backend, Option<&dyn crate::harness::Harness>) -> R + Send + 'static,
    ) -> Result<R> {
        let (reply_tx, reply_rx) = mpsc::channel();
        let job = Job {
            principal: who
                .cloned()
                .unwrap_or_else(|| Principal::unattributed("session")),
            keyed: who.is_some(),
            harness: harness.clone(),
            run: Work::Backend(Box::new(move |b, h| {
                let _ = reply_tx.send(f(b, h));
            })),
        };
        self.tx.send(job).map_err(|_| gone())?;
        reply_rx.recv().map_err(|_| gone())
    }

    /// Run `f` on the owner thread and wait for its answer. `Backend` when the owner is gone or `f` panicked.
    pub(in crate::gateway) fn call<R: Send + 'static>(
        &self,
        (principal, harness): (&Principal, &Option<HarnessMaker>),
        f: impl FnOnce(&dyn Transport) -> R + Send + 'static,
    ) -> Result<R> {
        let (reply_tx, reply_rx) = mpsc::channel();
        let job = Job {
            principal: principal.clone(),
            keyed: true,
            harness: harness.clone(),
            run: Work::Transport(Box::new(move |t| {
                let _ = reply_tx.send(f(t));
            })),
        };
        self.tx.send(job).map_err(|_| gone())?;
        reply_rx.recv().map_err(|_| gone())
    }

    #[cfg(test)]
    pub(crate) fn run_for_tests<R: Send + 'static>(
        &self,
        f: impl FnOnce(&dyn Transport) -> R + Send + 'static,
    ) -> Result<R> {
        self.call((&Principal::verified("test"), &None), f)
    }
}

/// The session loop's view of a shared gateway (`SharedGateway::session_backend`). `Clone`, `Send` and `Sync`.
#[derive(Clone)]
pub struct SharedBackend {
    gateway: SharedGateway,
    harness: Option<HarnessMaker>,
}

impl SharedBackend {
    /// The harness this connection runs, learned by the daemon's edge from evidence it checked (#877, #1096): a `join`
    /// is judged by it, not by the generic floor. `make` runs on the owner thread for each request. Nothing the peer
    /// sends is read for it.
    pub fn with_harness(
        mut self,
        make: impl Fn() -> Box<dyn crate::harness::Harness> + Send + Sync + 'static,
    ) -> Self {
        self.harness = Some(Arc::new(make));
        self
    }

    /// A request of the session loop: the frames that answer it, or the one failure frame when the owner is gone.
    fn failed(id: String, e: &Error) -> Vec<ServerFrame> {
        crate::session::failure_frames(id, e)
    }
}

impl Backend for SharedBackend {
    fn now(&self) -> i64 {
        self.gateway.call_backend(|b| b.now()).unwrap_or(0)
    }
    /// The store's token joined with the count of requests that may have committed here (the store's own does not move
    /// for the owner's commits), the way `SharedTransport::change_token` does.
    fn change_token(&self) -> Option<u64> {
        let store = self.gateway.call_backend(|b| b.change_token()).ok()??;
        let posts = self.gateway.posts.load(Ordering::SeqCst);
        Some(store.wrapping_add(posts.rotate_left(32)))
    }
    fn head(&self, who: &Principal, room: &str) -> Result<i64> {
        let (who, room) = (who.clone(), room.to_owned());
        let keyed = who.clone();
        self.gateway
            .call_backend_as((Some(&keyed), &None), move |b, _| b.head(&who, &room))?
    }
    fn ack(&self, who: &Principal, room: &str, seq: i64) -> Result<()> {
        let (who, room) = (who.clone(), room.to_owned());
        let keyed = who.clone();
        self.gateway
            .call_backend_as((Some(&keyed), &None), move |b, _| b.ack(&who, &room, seq))?
    }
    /// The subscription lives with the caller and goes to the owner for the length of one poll.
    fn poll(&self, sub: &mut Subscription, who: &Principal, max: usize) -> Result<Batch> {
        let mut moved = std::mem::replace(sub, Subscription::new("", 0, 0));
        let who = who.clone();
        let keyed = who.clone();
        let (batch, back) = self
            .gateway
            .call_backend_as((Some(&keyed), &None), move |b, _| {
                let batch = b.poll(&mut moved, &who, max);
                (batch, moved)
            })?;
        *sub = back;
        batch
    }
    fn handle(&self, who: &Principal, call: Call) -> Vec<ServerFrame> {
        let id = call.id.clone();
        let who = who.clone();
        let keyed = who.clone();
        let frames = self
            .gateway
            .call_backend_as((Some(&keyed), &self.harness), move |b, h| {
                b.handle_as(&who, call, h)
            });
        // any request may have stored something: a waiting session's change token moves after it
        self.gateway.posts.fetch_add(1, Ordering::SeqCst);
        frames.unwrap_or_else(|e| Self::failed(id, &e))
    }
    fn verify_credential(&self, member: &str, token: &str) -> Result<Principal> {
        let (member, token) = (member.to_owned(), token.to_owned());
        self.gateway
            .call_backend(move |b| b.verify_credential(&member, &token))?
    }
    fn note_member_fixed(&self, who: &Principal) {
        let who = who.clone();
        // the refusal stands whether or not the owner could keep the row
        let _ = self
            .gateway
            .call_backend(move |b| b.note_member_fixed(&who));
    }
}

fn gone() -> Error {
    Error::Backend("the gateway thread is not serving this request".into())
}

/// The client side of a shared gateway for one identity. `Clone`, `Send` and `Sync`.
#[derive(Clone)]
pub struct SharedTransport {
    pub(in crate::gateway) gateway: SharedGateway,
    pub(in crate::gateway) principal: Principal,
    pub(in crate::gateway) harness: Option<HarnessMaker>,
}

impl SharedTransport {
    /// The harness this caller runs, learned by the transport edge from evidence it checked (#877); see
    /// `Direct::with_harness`. `make` runs on the owner thread for each request.
    pub fn with_harness(
        mut self,
        make: impl Fn() -> Box<dyn crate::harness::Harness> + Send + Sync + 'static,
    ) -> Self {
        self.harness = Some(Arc::new(make));
        self
    }

    pub(in crate::gateway) fn call<R: Send + 'static>(
        &self,
        f: impl FnOnce(&dyn Transport) -> Result<R> + Send + 'static,
    ) -> Result<R> {
        self.gateway
            .call((&self.principal, &self.harness), f)
            .and_then(|r| r)
    }

    /// A call that stores a message: the posts count moves after it, so a waiter's change token moves.
    pub(in crate::gateway) fn posting<R: Send + 'static>(
        &self,
        f: impl FnOnce(&dyn Transport) -> Result<R> + Send + 'static,
    ) -> Result<R> {
        let r = self.call(f);
        self.gateway.posts.fetch_add(1, Ordering::SeqCst);
        r
    }
}

/// An argument that can be copied to the owner thread: the owned form, and how to lend the borrowed form back.
pub(in crate::gateway) trait Own {
    type Owned: Send + 'static;
    fn own(&self) -> Self::Owned;
}

pub(in crate::gateway) trait Lend<'a> {
    type Borrowed;
    fn lend(&'a self) -> Self::Borrowed;
}

impl Own for &str {
    type Owned = String;
    fn own(&self) -> String {
        (*self).to_owned()
    }
}
impl<'a> Lend<'a> for String {
    type Borrowed = &'a str;
    fn lend(&'a self) -> &'a str {
        self
    }
}
impl Own for Option<&str> {
    type Owned = Option<String>;
    fn own(&self) -> Option<String> {
        self.map(str::to_owned)
    }
}
impl<'a> Lend<'a> for Option<String> {
    type Borrowed = Option<&'a str>;
    fn lend(&'a self) -> Option<&'a str> {
        self.as_deref()
    }
}

macro_rules! by_copy {
    ($($t:ty),*) => {$(
        impl Own for $t {
            type Owned = $t;
            fn own(&self) -> $t {
                *self
            }
        }
        impl<'a> Lend<'a> for $t {
            type Borrowed = $t;
            fn lend(&'a self) -> $t {
                *self
            }
        }
    )*};
}
by_copy!(
    i64,
    usize,
    bool,
    Option<i64>,
    Option<(usize, i64)>,
    HistoryRange,
    crate::gateway::DeliveryView,
    crate::store::Shown
);
impl Own for &[i64] {
    type Owned = Vec<i64>;
    fn own(&self) -> Vec<i64> {
        self.to_vec()
    }
}
impl<'a> Lend<'a> for Vec<i64> {
    type Borrowed = &'a [i64];
    fn lend(&'a self) -> &'a [i64] {
        self
    }
}
impl Own for Vec<crate::domain::Message> {
    type Owned = Vec<crate::domain::Message>;
    fn own(&self) -> Vec<crate::domain::Message> {
        self.clone()
    }
}
impl<'a> Lend<'a> for Vec<crate::domain::Message> {
    type Borrowed = Vec<crate::domain::Message>;
    fn lend(&'a self) -> Vec<crate::domain::Message> {
        self.clone()
    }
}

/// Forward each listed method: copy the arguments, run the same method on the owner's `Direct`.
macro_rules! forward {
    ($($name:ident($($a:ident: $t:ty),*) -> $ret:ty;)*) => {$(
        fn $name(&self $(, $a: $t)*) -> $ret {
            $(let $a = $crate::gateway::shared::Own::own(&$a);)*
            self.call(move |t| t.$name($($crate::gateway::shared::Lend::lend(&$a)),*))
        }
    )*};
}
pub(in crate::gateway) use forward;
