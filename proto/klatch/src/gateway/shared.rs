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
use crate::gateway::{
    Batch, ClaimSpec, Direct, Gateway, Handoff, HistoryRange, Subscription, Transport,
};
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

/// What the owner runs for one session-backend request: the gateway itself, as the session loop sees it.
type BackendRun = Box<dyn FnOnce(&dyn Backend) + Send>;

enum Work {
    /// A `Transport` call as one principal.
    Transport(Run),
    /// A call of the session loop (`session::Backend`).
    Backend(BackendRun),
}

struct Job {
    principal: Principal,
    harness: Option<HarnessMaker>,
    run: Work,
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
    posts: Arc<AtomicU64>,
    _owner: Arc<Owner>,
}

impl SharedGateway {
    /// Start the owner thread and build the gateway on it with `build`, so nothing in the gateway has to be `Send`
    /// (open the store inside `build`). Returns what `build` failed with, or `Backend` if it panicked.
    pub fn spawn<S: Store + 'static>(
        build: impl FnOnce() -> Result<Gateway<S>> + Send + 'static,
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
                        let _ = catch_unwind(AssertUnwindSafe(|| run(&*gateway)));
                    }
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
        }
    }

    /// Run `f` on the owner thread against the gateway itself.
    fn call_backend<R: Send + 'static>(
        &self,
        f: impl FnOnce(&dyn Backend) -> R + Send + 'static,
    ) -> Result<R> {
        let (reply_tx, reply_rx) = mpsc::channel();
        let job = Job {
            principal: Principal::unattributed("session"),
            harness: None,
            run: Work::Backend(Box::new(move |b| {
                let _ = reply_tx.send(f(b));
            })),
        };
        self.tx.send(job).map_err(|_| gone())?;
        reply_rx.recv().map_err(|_| gone())
    }

    /// Run `f` on the owner thread and wait for its answer. `Backend` when the owner is gone or `f` panicked.
    fn call<R: Send + 'static>(
        &self,
        (principal, harness): (&Principal, &Option<HarnessMaker>),
        f: impl FnOnce(&dyn Transport) -> R + Send + 'static,
    ) -> Result<R> {
        let (reply_tx, reply_rx) = mpsc::channel();
        let job = Job {
            principal: principal.clone(),
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
}

impl SharedBackend {
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
        self.gateway.call_backend(move |b| b.head(&who, &room))?
    }
    fn ack(&self, who: &Principal, room: &str, seq: i64) -> Result<()> {
        let (who, room) = (who.clone(), room.to_owned());
        self.gateway
            .call_backend(move |b| b.ack(&who, &room, seq))?
    }
    /// The subscription lives with the caller and goes to the owner for the length of one poll.
    fn poll(&self, sub: &mut Subscription, who: &Principal, max: usize) -> Result<Batch> {
        let mut moved = std::mem::replace(sub, Subscription::new("", 0, 0));
        let who = who.clone();
        let (batch, back) = self.gateway.call_backend(move |b| {
            let batch = b.poll(&mut moved, &who, max);
            (batch, moved)
        })?;
        *sub = back;
        batch
    }
    fn handle(&self, who: &Principal, call: Call) -> Vec<ServerFrame> {
        let id = call.id.clone();
        let who = who.clone();
        let frames = self.gateway.call_backend(move |b| b.handle(&who, call));
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
    gateway: SharedGateway,
    principal: Principal,
    harness: Option<HarnessMaker>,
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

    fn call<R: Send + 'static>(
        &self,
        f: impl FnOnce(&dyn Transport) -> Result<R> + Send + 'static,
    ) -> Result<R> {
        self.gateway
            .call((&self.principal, &self.harness), f)
            .and_then(|r| r)
    }

    /// A call that stores a message: the posts count moves after it, so a waiter's change token moves.
    fn posting<R: Send + 'static>(
        &self,
        f: impl FnOnce(&dyn Transport) -> Result<R> + Send + 'static,
    ) -> Result<R> {
        let r = self.call(f);
        self.gateway.posts.fetch_add(1, Ordering::SeqCst);
        r
    }
}

/// An argument that can be copied to the owner thread: the owned form, and how to lend the borrowed form back.
trait Own {
    type Owned: Send + 'static;
    fn own(&self) -> Self::Owned;
}

trait Lend<'a> {
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
    crate::gateway::DeliveryView
);

/// Forward each listed method: copy the arguments, run the same method on the owner's `Direct`.
macro_rules! forward {
    ($($name:ident($($a:ident: $t:ty),*) -> $ret:ty;)*) => {$(
        fn $name(&self $(, $a: $t)*) -> $ret {
            $(let $a = Own::own(&$a);)*
            self.call(move |t| t.$name($(Lend::lend(&$a)),*))
        }
    )*};
}

impl Transport for SharedTransport {
    forward! {
        join(room: &str) -> Result<()>;
        read(room: &str, advance: bool) -> Result<Vec<crate::domain::Message>>;
        history(room: &str, range: HistoryRange) -> Result<crate::gateway::History>;
        wait_poll(room: &str) -> Result<Vec<crate::domain::Message>>;
        wakeable(room: &str) -> Result<usize>;
        wake_summary(room: &str) -> Result<(usize, i64)>;
        outside_view(room: &str) -> Result<Option<(usize, i64)>>;
        wake_mark(room: &str) -> Result<()>;
        wait_seen(room: &str, seq: i64) -> Result<()>;
        who(room: &str) -> Result<Vec<crate::domain::MemberInfo>>;
        receipt(room: &str, seq: i64) -> Result<crate::domain::Receipts>;
        set_scope(scope: &str) -> Result<()>;
        scope() -> Result<Option<String>>;
        release(room: &str, path: &str) -> Result<()>;
        release_with_note(room: &str, path: &str, note: &str) -> Result<()>;
        last_release(room: &str, path: &str) -> Result<Option<crate::domain::ReleaseNote>>;
        release_all() -> Result<usize>;
        end_credential() -> Result<bool>;
        admin_close_room(room: &str, closed: bool) -> Result<()>;
        admin_pause(paused: bool) -> Result<()>;
        admin_remove_member(room: &str, member: &str) -> Result<()>;
        admin_allow_member(room: &str, member: &str) -> Result<()>;
        admin_audit(limit: usize) -> Result<Vec<crate::domain::AuditRecord>>;
        admin_run_create(run: &str) -> Result<()>;
        admin_run_end(run: &str) -> Result<crate::store::DeletedRooms>;
        anchor_done(anchor: &str) -> Result<usize>;
        admin_set_room_label(room: &str, label: &str) -> Result<()>;
        admin_set_clearance(member: &str, label: &str) -> Result<()>;
        admin_set_cap(label: &str) -> Result<()>;
        admin_set_librarian(member: &str, on: bool) -> Result<()>;
        admin_set_reviewer(member: &str, on: bool) -> Result<()>;
        admin_set_facilitator(member: &str, on: bool) -> Result<()>;
        admin_set_integrator(member: &str, on: bool) -> Result<()>;
        admin_set_ssh_key(label: &str, member: &str, on: bool) -> Result<()>;
        admin_set_admin_agent(member: &str, on: bool) -> Result<()>;
        admin_set_room_creation_granted(on: bool) -> Result<()>;
        admin_grant_room_capability(member: &str, on: bool) -> Result<()>;
        invite(member: &str, room: &str, via: &str) -> Result<()>;
        grant_room_clearance(room: &str, member: &str, label: &str) -> Result<()>;
        admin_set_room_grantor(room: &str, member: &str, on: bool) -> Result<()>;
        admin_set_room_clearance(room: &str, member: &str, label: Option<&str>) -> Result<()>;
        admin_set_rate_limit(limit: Option<(usize, i64)>) -> Result<()>;
        admin_set_member_rate_limit(member: &str, limit: Option<(usize, i64)>) -> Result<()>;
        admin_rate_limits() -> Result<Vec<crate::gateway::RateLimitEntry>>;
        admin_set_lease_cap(secs: Option<i64>) -> Result<()>;
        admin_set_lease_grace(secs: Option<i64>) -> Result<()>;
        admin_set_room_stale(room: &str, secs: Option<i64>) -> Result<()>;
        admin_set_room_ask_lease(room: &str, secs: Option<i64>) -> Result<()>;
        admin_set_room_require(room: &str, spec: Option<&str>) -> Result<()>;
        claim_ttl(room: &str, path: &str, requested: Option<i64>) -> Result<i64>;
        admin_plant_canary(room: &str) -> Result<()>;
        admin_retire_canary(room: &str) -> Result<()>;
        admin_add_scrub_term(name: &str, term: &str) -> Result<()>;
        admin_remove_scrub_term(name: &str) -> Result<()>;
        admin_scrub_terms() -> Result<Vec<(String, String)>>;
        admin_refusals(limit: usize) -> Result<Vec<crate::domain::RefusalRecord>>;
        my_refusals(limit: usize) -> Result<Vec<crate::domain::RefusalRecord>>;
        my_crossings(limit: usize) -> Result<Vec<crate::domain::CrossingStatus>>;
        review_next() -> Result<Option<crate::domain::ReviewItem>>;
        claims(room: &str) -> Result<Vec<crate::domain::Claim>>;
        digest() -> Result<crate::gateway::Digest>;
        digest_with(backlog: bool) -> Result<crate::gateway::Digest>;
        record_hook_run(event: &str, bytes: usize) -> Result<()>;
        hook_runs(member: &str) -> Result<Vec<crate::domain::HookRun>>;
        note_member_mode(mode: &str) -> Result<()>;
        note_session(session_id: &str) -> Result<()>;
        record_stop(background_tasks: usize, crons: usize) -> Result<()>;
        sign_of_life() -> Result<()>;
        prune_members(older_than: i64) -> Result<usize>;
        set_status(room: &str, status: &str) -> Result<()>;
        rooms() -> Result<Vec<String>>;
        set_muted(room: &str, muted: bool) -> Result<()>;
        delivery_view() -> Result<crate::gateway::DeliveryView>;
        set_delivery_view(view: crate::gateway::DeliveryView) -> Result<()>;
        request_crossing(from: &str, seq: i64, to: &str) -> Result<i64>;
        review_queue(limit: usize) -> Result<Vec<crate::store::QueuedReview>>;
        board(room: &str) -> Result<crate::domain::Board>;
        member_card(room: &str, member: &str) -> Result<Option<crate::domain::Card>>;
        open_asks(room: &str) -> Result<Vec<crate::domain::OpenAsk>>;
        stuck(room: &str) -> Result<Vec<crate::domain::StuckEvent>>;
        process_exited(room: &str, member: &str) -> Result<bool>;
        ack(room: &str, seq: i64) -> Result<()>;
    }

    // The calls that store a message also move the posts count, so a waiter's change token moves.
    fn dm(
        &self,
        to: &str,
        body: &str,
        label: Option<&str>,
    ) -> Result<(String, crate::domain::Posted)> {
        let (to, body, label) = (Own::own(&to), Own::own(&body), Own::own(&label));
        self.posting(move |t| t.dm(Lend::lend(&to), Lend::lend(&body), Lend::lend(&label)))
    }
    fn post_labeled(
        &self,
        room: &str,
        body: &str,
        label: Option<&str>,
    ) -> Result<crate::domain::Posted> {
        let (room, body, label) = (Own::own(&room), Own::own(&body), Own::own(&label));
        self.posting(move |t| {
            t.post_labeled(Lend::lend(&room), Lend::lend(&body), Lend::lend(&label))
        })
    }
    fn post_handoff(
        &self,
        (room, body, label): (&str, &str, Option<&str>),
        handoff: &Handoff,
    ) -> Result<crate::domain::Posted> {
        let (room, body, label) = (Own::own(&room), Own::own(&body), Own::own(&label));
        let (kind, payload) = (Own::own(&handoff.kind), Own::own(&handoff.payload));
        self.posting(move |t| {
            let handoff = Handoff {
                kind: Lend::lend(&kind),
                payload: Lend::lend(&payload),
            };
            t.post_handoff(
                (Lend::lend(&room), Lend::lend(&body), Lend::lend(&label)),
                &handoff,
            )
        })
    }

    fn claim(&self, room: &str, spec: &ClaimSpec) -> Result<()> {
        let room = Own::own(&room);
        let (path, reason, anchor) = (
            Own::own(&spec.path),
            Own::own(&spec.reason),
            Own::own(&spec.anchor),
        );
        let (ttl, shared) = (spec.ttl, spec.shared);
        self.call(move |t| {
            let mut spec = ClaimSpec::new(Lend::lend(&path), ttl, Lend::lend(&reason));
            spec.shared = shared;
            spec.anchor = Lend::lend(&anchor);
            t.claim(Lend::lend(&room), &spec)
        })
    }
    fn admin_set_label_chain(&self, names: &[&str]) -> Result<()> {
        let names: Vec<String> = names.iter().map(|n| (*n).to_owned()).collect();
        self.call(move |t| {
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            t.admin_set_label_chain(&names)
        })
    }
    fn review_verdict(&self, id: i64, verdict: &crate::domain::Verdict) -> Result<()> {
        let verdict = verdict.clone();
        self.call(move |t| t.review_verdict(id, &verdict))
    }

    /// The store's own token (it moves when another process commits) joined with the count of posts made through the
    /// handles (the store's does not move for the owner's own commits). `None` when the store has none.
    fn change_token(&self) -> Option<u64> {
        let store = self
            .gateway
            .call((&self.principal, &None), |t| t.change_token())
            .ok()
            .flatten()?;
        let posts = self.gateway.posts.load(Ordering::SeqCst);
        Some(store.wrapping_add(posts.rotate_left(32)))
    }

    fn subscribe(&self, room: &str, from_seq: i64) -> Result<Subscription> {
        let room = Own::own(&room);
        self.call(move |t| t.subscribe(Lend::lend(&room), from_seq))
    }
    /// The subscription lives with the caller and goes to the owner for the length of one poll.
    fn poll(&self, subscription: &mut Subscription, max: usize) -> Result<Batch> {
        let mut moved = std::mem::replace(subscription, Subscription::new("", 0, 0));
        let r = self
            .gateway
            .call((&self.principal, &self.harness), move |t| {
                let batch = t.poll(&mut moved, max);
                (batch, moved)
            });
        match r {
            Ok((batch, back)) => {
                *subscription = back;
                batch
            }
            Err(e) => Err(e),
        }
    }
}
